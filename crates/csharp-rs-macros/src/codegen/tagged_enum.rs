// Rust guideline compliant 2026-02-11
//! Tagged enum code generation (internally, adjacently, externally tagged,
//! and untagged).
//!
//! Generates C# abstract record type hierarchies with sealed derived records.
//! Dispatches to the appropriate code generation strategy based on the
//! [`EnumTagging`] variant and the target C# version / serializer combination.

use crate::config::{CSharpConfig, CSharpVersion, Serializer};
use crate::types::{CSharpField, EnumTagging, TaggedVariant, TaggedVariantData};
use proc_macro2::TokenStream;
use quote::quote;

/// Builds a tagged enum definition token stream.
///
/// Dispatches to the appropriate code generation strategy based on the
/// [`EnumTagging`] variant (internal, adjacent, external, or untagged).
///
/// The returned token stream evaluates at compile time to produce the complete
/// C# file contents including using directives, namespace, polymorphic
/// attributes, and nested sealed record declarations.
pub fn build_tagged_enum_definition(
    csharp_name: &str,
    namespace: &str,
    tagging: &EnumTagging,
    variants: &[TaggedVariant],
    config: &CSharpConfig,
) -> TokenStream {
    let use_file_scoped_ns = config.target >= CSharpVersion::CSharp10;
    let use_required = config.target >= CSharpVersion::CSharp11;
    let use_native_polymorphism = matches!(tagging, EnumTagging::Internal { .. })
        && config.serializer == Serializer::SystemTextJson
        && config.target >= CSharpVersion::CSharp11;

    let using_block = build_using_block(config, use_native_polymorphism);
    let class_attrs =
        build_class_attributes(csharp_name, tagging, variants, use_native_polymorphism);

    let variant_exprs = build_variant_exprs(
        csharp_name,
        variants,
        config,
        use_file_scoped_ns,
        use_required,
    );

    let indent = if use_file_scoped_ns { "" } else { "    " };

    // Build the format template depending on namespace style.
    if use_file_scoped_ns {
        build_file_scoped(
            &using_block,
            namespace,
            &class_attrs,
            csharp_name,
            indent,
            &variant_exprs,
        )
    } else {
        build_block_scoped(
            &using_block,
            namespace,
            &class_attrs,
            csharp_name,
            indent,
            &variant_exprs,
        )
    }
}

/// Determines the `using` directives for the generated file.
fn build_using_block(config: &CSharpConfig, use_native_polymorphism: bool) -> String {
    if use_native_polymorphism {
        // Native [JsonPolymorphic] path: only need Serialization namespace.
        return String::from("using System.Text.Json.Serialization;");
    }

    // Converter path (or non-internal tagging) — depends on serializer.
    match config.serializer {
        Serializer::SystemTextJson => String::from(
            "using System;\nusing System.Text.Json;\nusing System.Text.Json.Serialization;",
        ),
        Serializer::Newtonsoft => {
            String::from("using Newtonsoft.Json;\nusing Newtonsoft.Json.Linq;")
        }
    }
}

/// Builds the class-level attributes (polymorphic attrs or converter attr).
fn build_class_attributes(
    csharp_name: &str,
    tagging: &EnumTagging,
    variants: &[TaggedVariant],
    use_native_polymorphism: bool,
) -> String {
    use std::fmt::Write;

    if use_native_polymorphism {
        // Native [JsonPolymorphic] + [JsonDerivedType] attributes.
        let EnumTagging::Internal { tag } = tagging else {
            unreachable!("native polymorphism only for internal tagging")
        };

        let mut attrs = format!("[JsonPolymorphic(TypeDiscriminatorPropertyName = \"{tag}\")]\n");
        for v in variants {
            writeln!(
                attrs,
                "[JsonDerivedType(typeof({name}.{variant}), \"{json}\")]",
                name = csharp_name,
                variant = v.csharp_name,
                json = v.json_name,
            )
            .expect("writing to String never fails");
        }
        // Remove trailing newline.
        if attrs.ends_with('\n') {
            attrs.pop();
        }
        return attrs;
    }

    // Converter path — both serializers use the same [JsonConverter] attribute.
    format!("[JsonConverter(typeof({csharp_name}Converter))]")
}

/// Builds token stream expressions for each variant's C# nested record.
///
/// Returns a `Vec<TokenStream>` where each element produces a `String` at
/// runtime containing the complete nested record declaration for one variant.
fn build_variant_exprs(
    parent_name: &str,
    variants: &[TaggedVariant],
    config: &CSharpConfig,
    use_file_scoped_ns: bool,
    use_required: bool,
) -> Vec<TokenStream> {
    // Base indent: inside the abstract record body.
    // File-scoped: 4 spaces for record body members.
    // Block-scoped: 8 spaces (4 for namespace + 4 for record body).
    let base_indent = if use_file_scoped_ns {
        "    "
    } else {
        "        "
    };

    variants
        .iter()
        .map(|v| build_single_variant(parent_name, v, config, base_indent, use_required))
        .collect()
}

/// Builds the token stream for a single variant's nested sealed record.
fn build_single_variant(
    parent_name: &str,
    variant: &TaggedVariant,
    config: &CSharpConfig,
    base_indent: &str,
    use_required: bool,
) -> TokenStream {
    let variant_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            // Unit variant: `public sealed record Quit : Message;`
            let line = format!("{base_indent}public sealed record {variant_name} : {parent_name};");
            quote! { String::from(#line) }
        }
        TaggedVariantData::Newtype { type_expr } => {
            // Newtype variant: single `Value` property.
            let attr_name = json_attr_name(config);
            let required_kw = if use_required { "required " } else { "" };
            let prop_indent = format!("{base_indent}    ");

            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{base}public sealed record {name} : {parent}\n\
                         {base}{{\n\
                         {prop}[{attr}(\"Value\")]\n\
                         {prop}public {req}{ty} Value {{ get; init; }}\n\
                         {base}}}",
                        base = #base_indent,
                        name = #variant_name,
                        parent = #parent_name,
                        prop = #prop_indent,
                        attr = #attr_name,
                        req = #required_kw,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_struct_variant(
            parent_name,
            variant_name,
            fields,
            config,
            base_indent,
            use_required,
        ),
    }
}

/// Builds the token stream for a struct variant with named fields.
fn build_struct_variant(
    parent_name: &str,
    variant_name: &str,
    fields: &[CSharpField],
    config: &CSharpConfig,
    base_indent: &str,
    use_required: bool,
) -> TokenStream {
    let attr_name = json_attr_name(config);
    let prop_indent = format!("{base_indent}    ");
    let required_kw = if use_required { "required " } else { "" };

    let field_exprs: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let prop_name = &f.csharp_property_name;
            let json_name = &f.json_name;
            let is_optional = f.is_optional;
            let type_expr = &f.type_expr;

            quote! {
                {
                    let csharp_type = #type_expr;
                    let nullable = if #is_optional { "?" } else { "" };
                    let req = if #is_optional { "" } else { #required_kw };
                    format!(
                        "{prop}[{attr}(\"{json}\")]\n\
                         {prop}public {req}{ty}{null} {name} {{ get; init; }}",
                        prop = #prop_indent,
                        attr = #attr_name,
                        json = #json_name,
                        req = req,
                        ty = csharp_type,
                        null = nullable,
                        name = #prop_name,
                    )
                }
            }
        })
        .collect();

    quote! {
        {
            let field_parts: Vec<String> = vec![#(#field_exprs),*];
            let fields_block = field_parts.join("\n");
            format!(
                "{base}public sealed record {name} : {parent}\n\
                 {base}{{\n\
                 {fields}\n\
                 {base}}}",
                base = #base_indent,
                name = #variant_name,
                parent = #parent_name,
                fields = fields_block,
            )
        }
    }
}

/// Returns the JSON attribute name for a property based on the serializer.
fn json_attr_name(config: &CSharpConfig) -> &'static str {
    match config.serializer {
        Serializer::SystemTextJson => "JsonPropertyName",
        Serializer::Newtonsoft => "JsonProperty",
    }
}

/// Builds the final token stream for file-scoped namespace output (C# 10+).
fn build_file_scoped(
    using_block: &str,
    namespace: &str,
    class_attrs: &str,
    csharp_name: &str,
    indent: &str,
    variant_exprs: &[TokenStream],
) -> TokenStream {
    quote! {
        {
            let variant_parts: Vec<String> = vec![#(#variant_exprs),*];
            let variants_block = variant_parts.join("\n\n");

            let body = if variants_block.is_empty() {
                String::from(";")
            } else {
                format!("\n{{\n{variants}\n}}", variants = variants_block)
            };

            format!(
                "// <auto-generated/>\n\
                 {using}\n\
                 \n\
                 namespace {ns};\n\
                 \n\
                 {attrs}\n\
                 {indent}public abstract record {name}{body}\n",
                using = #using_block,
                ns = #namespace,
                attrs = #class_attrs,
                indent = #indent,
                name = #csharp_name,
                body = body,
            )
        }
    }
}

/// Builds the final token stream for block-scoped namespace output (C# 9).
fn build_block_scoped(
    using_block: &str,
    namespace: &str,
    class_attrs: &str,
    csharp_name: &str,
    indent: &str,
    variant_exprs: &[TokenStream],
) -> TokenStream {
    quote! {
        {
            let variant_parts: Vec<String> = vec![#(#variant_exprs),*];
            let variants_block = variant_parts.join("\n\n");

            let body = if variants_block.is_empty() {
                String::from(";")
            } else {
                format!("\n    {{\n{variants}\n    }}", variants = variants_block)
            };

            format!(
                "// <auto-generated/>\n\
                 {using}\n\
                 \n\
                 namespace {ns}\n\
                 {{\n\
                 {indent}{attrs}\n\
                 {indent}public abstract record {name}{body}\n\
                 }}\n",
                using = #using_block,
                ns = #namespace,
                indent = #indent,
                attrs = #class_attrs,
                name = #csharp_name,
                body = body,
            )
        }
    }
}
