// Rust guideline compliant 2026-03-15
//! Tagged enum code generation (internally, adjacently, externally tagged, and
//! untagged).
//!
//! Generates C# abstract record type hierarchies with sealed derived records.
//! Dispatches to the appropriate code generation strategy based on the
//! [`EnumTagging`] variant and the target C# version / serializer combination.
//!
//! All configuration decisions (serializer library, C# version) are deferred
//! to runtime via `cfg: &csharp_rs::Config`, which is in scope in the
//! generated `csharp_definition(cfg)` method body.
//!
//! When native polymorphism is not available (STJ C# 9-10, or Newtonsoft at
//! any version), a custom `JsonConverter<T>` is generated as a nested
//! `private sealed class` inside the abstract record.

use crate::types::{CSharpField, EnumTagging, FlattenKind, TaggedVariant, TaggedVariantData};
use proc_macro2::TokenStream;
use quote::quote;

/// C# reserved keywords that cannot be used as identifiers without escaping.
///
/// Pattern variables in `case` arms use the lowercased variant name, which can
/// collide with C# keywords (e.g. variant `Float` → `float`). Prefixing with
/// `@` produces a valid verbatim identifier (`@float`).
const CSHARP_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

/// Returns a C# safe identifier for use as a pattern variable name.
///
/// Lowercases the input and prefixes it with `@` if it collides with a C#
/// reserved keyword. For example, `"Float"` becomes `"@float"` and
/// `"Request"` becomes `"request"`.
fn csharp_safe_var_name(name: &str) -> String {
    let lower = name.to_lowercase();
    if CSHARP_KEYWORDS.contains(&lower.as_str()) {
        format!("@{lower}")
    } else {
        lower
    }
}

/// Builds a tagged enum definition token stream.
///
/// Dispatches to the appropriate code generation strategy based on the
/// [`EnumTagging`] variant (internal, adjacent, external, or untagged).
///
/// The returned token stream evaluates at runtime to produce the complete
/// C# file contents including using directives, namespace, polymorphic
/// attributes, nested sealed record declarations, and optionally a custom
/// `JsonConverter<T>` class when native polymorphism is not available.
///
/// All configuration decisions (serializer library, C# version) are deferred
/// to runtime via `cfg: &csharp_rs::Config`, which is in scope in the
/// generated `csharp_definition(cfg)` method body.
///
/// Version-dependent features:
/// - **C# 10+**: file-scoped namespace (`namespace X;` instead of block).
/// - **C# 11+**: `required` modifier on non-optional properties.
/// - **C# 11+ with STJ and internal tagging**: native polymorphism via
///   `[JsonPolymorphic]` / `[JsonDerivedType]` attributes instead of a
///   custom `JsonConverter<T>`.
#[expect(
    clippy::too_many_lines,
    reason = "orchestrates all tagged enum codegen branches"
)]
pub fn build_tagged_enum_definition(
    csharp_name: &str,
    ns_expr: &TokenStream,
    tagging: &EnumTagging,
    variants: &[TaggedVariant],
) -> TokenStream {
    let is_internal = matches!(tagging, EnumTagging::Internal { .. });

    // A HashMap flatten field always emits a nullable `Dictionary<…>?
    // ExtensionData` reference-type property, so the file needs `#nullable
    // enable` unconditionally when any variant contains one.
    let has_hashmap_flatten = variants.iter().any(|v| match &v.data {
        TaggedVariantData::Struct(fields) => fields
            .iter()
            .any(|f| matches!(f.flatten, FlattenKind::HashMap { .. })),
        _ => false,
    });

    let class_attrs_expr = build_class_attributes_expr(csharp_name, tagging, variants);

    let variant_exprs = build_variant_exprs(csharp_name, variants);

    let converter_expr = build_converter_block(csharp_name, tagging, variants);

    // Collect variant C# names for qualifying references when the converter
    // is moved outside the record body (generic types).
    let variant_csharp_names: Vec<&str> = variants.iter().map(|v| v.csharp_name.as_str()).collect();

    // Collect nullable-reference-type checks from all struct variant fields.
    let nullable_checks: Vec<TokenStream> = variants
        .iter()
        .flat_map(|v| match &v.data {
            TaggedVariantData::Struct(fields) => super::nullable_ref_check_exprs(fields),
            _ => Vec::new(),
        })
        .collect();

    let converter_append = quote! { #converter_expr };

    quote! {
        {
            let ns: &str = #ns_expr;

            // Runtime version selection
            let use_file_scoped = cfg.target() >= csharp_rs::CSharpVersion::CSharp10;
            let use_required = cfg.target() >= csharp_rs::CSharpVersion::CSharp11;
            let use_native_polymorphism = #is_internal
                && matches!(cfg.serializer(), csharp_rs::Serializer::SystemTextJson)
                && cfg.target() >= csharp_rs::CSharpVersion::CSharp11
                && generic_suffix.is_empty();

            let required_kw = if use_required { "required " } else { "" };
            let type_kw = if cfg.target().uses_records() { "record" } else { "class" };
            let accessor = if cfg.target().uses_records() { "init" } else { "set" };
            let base_indent = if use_file_scoped { "    " } else { "        " };
            let prop_indent = format!("{}    ", base_indent);

            // Runtime serializer selection
            let attr_name = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => "JsonPropertyName",
                csharp_rs::Serializer::Newtonsoft => "JsonProperty",
            };
            let extension_data_type = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => "Dictionary<string, JsonElement>",
                csharp_rs::Serializer::Newtonsoft => "Dictionary<string, JToken>",
            };

            // Using directives
            let using_block = if use_native_polymorphism {
                if #has_hashmap_flatten {
                    "using System;\nusing System.Collections.Generic;\nusing System.Linq;\nusing System.Text.Json;\nusing System.Text.Json.Serialization;"
                } else {
                    "using System;\nusing System.Collections.Generic;\nusing System.Linq;\nusing System.Text.Json.Serialization;"
                }
            } else {
                match cfg.serializer() {
                    csharp_rs::Serializer::SystemTextJson => {
                        "using System;\nusing System.Collections.Generic;\nusing System.Linq;\nusing System.Text.Json;\nusing System.Text.Json.Serialization;"
                    }
                    csharp_rs::Serializer::Newtonsoft => {
                        "using System;\nusing System.Collections.Generic;\nusing System.Linq;\nusing Newtonsoft.Json;\nusing Newtonsoft.Json.Linq;"
                    }
                }
            };

            // Class attributes
            let class_attrs: String = #class_attrs_expr;

            // Variants
            let variant_parts: Vec<String> = vec![#(#variant_exprs),*];
            let variants_block = variant_parts.join("\n\n");

            // Converter
            let converter_block: String = #converter_append;

            // Factory / adapter for generic types: a non-generic class that
            // can be referenced by `[JsonConverter(typeof(...))]` without
            // triggering CS0416, and delegates to the generic converter.
            //
            // STJ: `{Name}ConverterFactory : JsonConverterFactory`
            // Newtonsoft: `{Name}Converter : JsonConverter` (delegates to
            //             `{Name}Converter<T>` via reflection)
            let factory_block: String = if !generic_suffix.is_empty() {
                let open_commas = ",".repeat(generic_suffix.matches(',').count());
                let fbase = if use_file_scoped { "" } else { "    " };
                let fi1 = format!("{fbase}    ");
                let fi2 = format!("{fbase}        ");
                match cfg.serializer() {
                    csharp_rs::Serializer::SystemTextJson => {
                        format!(
                            "\n\
                             {base}internal sealed class {name}ConverterFactory : JsonConverterFactory\n\
                             {base}{{\n\
                             {i1}public override bool CanConvert(Type typeToConvert) =>\n\
                             {i2}typeToConvert.IsGenericType && typeToConvert.GetGenericTypeDefinition() == typeof({name}<{oc}>);\n\
                             \n\
                             {i1}public override JsonConverter CreateConverter(Type typeToConvert, JsonSerializerOptions options)\n\
                             {i1}{{\n\
                             {i2}var typeArgs = typeToConvert.GenericTypeArguments;\n\
                             {i2}var converterType = typeof({name}Converter<{oc}>).MakeGenericType(typeArgs);\n\
                             {i2}return (JsonConverter)Activator.CreateInstance(converterType)!;\n\
                             {i1}}}\n\
                             {base}}}",
                            base = fbase,
                            name = #csharp_name,
                            oc = open_commas,
                            i1 = fi1,
                            i2 = fi2,
                        )
                    }
                    csharp_rs::Serializer::Newtonsoft => {
                        format!(
                            "\n\
                             {base}internal sealed class {name}Converter : JsonConverter\n\
                             {base}{{\n\
                             {i1}public override bool CanConvert(Type objectType) =>\n\
                             {i2}objectType.IsGenericType && objectType.GetGenericTypeDefinition() == typeof({name}<{oc}>);\n\
                             \n\
                             {i1}public override object ReadJson(\n\
                             {i2}JsonReader reader,\n\
                             {i2}Type objectType,\n\
                             {i2}object existingValue,\n\
                             {i2}JsonSerializer serializer)\n\
                             {i1}{{\n\
                             {i2}var typeArgs = objectType.GenericTypeArguments;\n\
                             {i2}var converterType = typeof({name}Converter<{oc}>).MakeGenericType(typeArgs);\n\
                             {i2}var inner = (JsonConverter)Activator.CreateInstance(converterType)!;\n\
                             {i2}return inner.ReadJson(reader, objectType, existingValue, serializer)!;\n\
                             {i1}}}\n\
                             \n\
                             {i1}public override void WriteJson(\n\
                             {i2}JsonWriter writer,\n\
                             {i2}object value,\n\
                             {i2}JsonSerializer serializer)\n\
                             {i1}{{\n\
                             {i2}var objectType = value.GetType();\n\
                             {i2}while (objectType != null && !(objectType.IsGenericType && objectType.GetGenericTypeDefinition() == typeof({name}<{oc}>)))\n\
                             {i2}    objectType = objectType.BaseType;\n\
                             {i2}var typeArgs = objectType!.GenericTypeArguments;\n\
                             {i2}var converterType = typeof({name}Converter<{oc}>).MakeGenericType(typeArgs);\n\
                             {i2}var inner = (JsonConverter)Activator.CreateInstance(converterType)!;\n\
                             {i2}inner.WriteJson(writer, value, serializer);\n\
                             {i1}}}\n\
                             {base}}}",
                            base = fbase,
                            name = #csharp_name,
                            oc = open_commas,
                            i1 = fi1,
                            i2 = fi2,
                        )
                    }
                }
            } else {
                String::new()
            };

            // For generic tagged enums, move converter classes outside
            // the record body to avoid CS0416 (typeof cannot reference
            // members of open generic types). The classes use `internal`
            // visibility instead of `private` at namespace level.
            //
            // For STJ generics: the converter is already generic
            //   (`{Name}Converter<T>`), so it works outside directly.
            // For Newtonsoft generics: the converter was non-generic
            //   (`{Name}Converter : JsonConverter`). We make it generic
            //   (`{Name}Converter<T> : JsonConverter`) so `T` stays in
            //   scope, and add a non-generic adapter that delegates via
            //   reflection (same pattern as the STJ ConverterFactory).
            let (converter_inside, converter_outside) = if !generic_suffix.is_empty() {
                // Generic: move converter outside the record body.
                // Adjust indentation: base_indent is record-member level, but
                // outside the record we need namespace level (type_indent).
                let ns_indent = if use_file_scoped { "" } else { "    " };
                let mut adjusted = converter_block
                    .replace("private sealed class", "internal sealed class")
                    .replace(&format!("\n{}", base_indent), &format!("\n{}", ns_indent));

                // For Newtonsoft generics, the converter class was non-generic
                // (using `object` params + `CanConvert`). Make it generic by
                // adding `<T>` so that `T` is in scope for the body.
                // The `CanConvert` / `object` signature stays but `T` is usable.
                if matches!(cfg.serializer(), csharp_rs::Serializer::Newtonsoft) {
                    adjusted = adjusted.replace(
                        &format!("class {}Converter : JsonConverter", #csharp_name),
                        &format!("class {}Converter<T> : JsonConverter", #csharp_name),
                    );
                }

                // Qualify nested variant type references: variant records are
                // still nested inside the generic parent type, so references
                // like `new Data` or `case Data` must become
                // `new GenericResponse<T>.Data` / `case GenericResponse<T>.Data`.
                let parent_qualified = format!("{}{}", #csharp_name, generic_suffix);
                let variant_names: Vec<&str> = vec![#(#variant_csharp_names),*];
                for vn in &variant_names {
                    adjusted = adjusted
                        .replace(
                            &format!("new {vn}"),
                            &format!("new {parent_qualified}.{vn}"),
                        )
                        .replace(
                            &format!("case {vn} "),
                            &format!("case {parent_qualified}.{vn} "),
                        )
                        .replace(
                            &format!("case {vn}:"),
                            &format!("case {parent_qualified}.{vn}:"),
                        );
                }

                (String::new(), format!("{adjusted}{factory_block}"))
            } else {
                // Non-generic: converter stays nested inside the record body.
                (format!("{converter_block}{factory_block}"), String::new())
            };

            let body = if variants_block.is_empty() && converter_inside.is_empty() {
                String::from(";")
            } else {
                if use_file_scoped {
                    format!(
                        "\n{{\n{variants}{converter}\n}}",
                        variants = variants_block,
                        converter = converter_inside,
                    )
                } else {
                    format!(
                        "\n    {{\n{variants}{converter}\n    }}",
                        variants = variants_block,
                        converter = converter_inside,
                    )
                }
            };

            let nullable_checks: Vec<bool> = vec![#(#nullable_checks),*];
            let nullable_directive = if #has_hashmap_flatten || nullable_checks.iter().any(|&x| x) {
                "#nullable enable\n"
            } else {
                ""
            };

            let type_indent = if use_file_scoped { "" } else { "    " };

            if use_file_scoped {
                format!(
                    "// <auto-generated/>\n\
                     {nullable}\
                     {using}\n\
                     \n\
                     namespace {ns};\n\
                     \n\
                     {attrs}\n\
                     public abstract {type_kw} {name}{generics}{body}\n\
                     {outside}",
                    nullable = nullable_directive,
                    using = using_block,
                    ns = ns,
                    attrs = class_attrs,
                    type_kw = type_kw,
                    name = #csharp_name,
                    generics = generic_suffix,
                    body = body,
                    outside = converter_outside,
                )
            } else {
                format!(
                    "// <auto-generated/>\n\
                     {nullable}\
                     {using}\n\
                     \n\
                     namespace {ns}\n\
                     {{\n\
                     {ti}{attrs}\n\
                     {ti}public abstract {type_kw} {name}{generics}{body}\n\
                     {outside}\
                     }}\n",
                    nullable = nullable_directive,
                    using = using_block,
                    ns = ns,
                    ti = type_indent,
                    attrs = class_attrs,
                    type_kw = type_kw,
                    name = #csharp_name,
                    generics = generic_suffix,
                    body = body,
                    outside = converter_outside,
                )
            }
        }
    }
}

/// Builds a token stream expression that evaluates to the class-level
/// attributes string at runtime.
///
/// For native polymorphism (STJ + internal tagging + C# 11+), emits
/// `[JsonPolymorphic]` + `[JsonDerivedType]` attributes. Otherwise emits
/// `[JsonConverter(typeof({Name}Converter))]`.
fn build_class_attributes_expr(
    csharp_name: &str,
    tagging: &EnumTagging,
    variants: &[TaggedVariant],
) -> TokenStream {
    // Build the native polymorphism attributes at compile time since they
    // only depend on variant names (known at macro expansion time).
    if let EnumTagging::Internal { tag } = tagging {
        let mut native_attrs =
            format!("[JsonPolymorphic(TypeDiscriminatorPropertyName = \"{tag}\")]\n");
        for v in variants {
            use std::fmt::Write;
            writeln!(
                native_attrs,
                "[JsonDerivedType(typeof({name}.{variant}), \"{json}\")]",
                name = csharp_name,
                variant = v.csharp_name,
                json = v.json_name,
            )
            .expect("writing to String never fails");
        }
        // Remove trailing newline.
        if native_attrs.ends_with('\n') {
            native_attrs.pop();
        }

        let converter_attr_non_generic = format!("[JsonConverter(typeof({csharp_name}Converter))]");
        let converter_factory_attr =
            format!("[JsonConverter(typeof({csharp_name}ConverterFactory))]");

        quote! {
            if use_native_polymorphism {
                String::from(#native_attrs)
            } else if !generic_suffix.is_empty() {
                match cfg.serializer() {
                    csharp_rs::Serializer::SystemTextJson => {
                        String::from(#converter_factory_attr)
                    }
                    csharp_rs::Serializer::Newtonsoft => {
                        String::from(#converter_attr_non_generic)
                    }
                }
            } else {
                String::from(#converter_attr_non_generic)
            }
        }
    } else {
        // Non-internal tagging always uses converter.
        let converter_attr_non_generic = format!("[JsonConverter(typeof({csharp_name}Converter))]");
        let converter_factory_attr =
            format!("[JsonConverter(typeof({csharp_name}ConverterFactory))]");
        quote! {
            if !generic_suffix.is_empty() {
                match cfg.serializer() {
                    csharp_rs::Serializer::SystemTextJson => {
                        String::from(#converter_factory_attr)
                    }
                    csharp_rs::Serializer::Newtonsoft => {
                        String::from(#converter_attr_non_generic)
                    }
                }
            } else {
                String::from(#converter_attr_non_generic)
            }
        }
    }
}

/// Builds token stream expressions for each variant's C# nested record.
///
/// Returns a `Vec<TokenStream>` where each element produces a `String` at
/// runtime containing the complete nested record declaration for one variant.
///
/// References runtime variables `base_indent`, `prop_indent`, `attr_name`,
/// `required_kw`, and `extension_data_type` which must be in scope.
fn build_variant_exprs(parent_name: &str, variants: &[TaggedVariant]) -> Vec<TokenStream> {
    variants
        .iter()
        .map(|v| build_single_variant(parent_name, v))
        .collect()
}

/// Builds the token stream for a single variant's nested sealed record.
///
/// References runtime variables `base_indent`, `prop_indent`, `attr_name`,
/// and `required_kw` which must be in scope from the enclosing block.
fn build_single_variant(parent_name: &str, variant: &TaggedVariant) -> TokenStream {
    let variant_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            // Unit variant: `public sealed record Quit : Message;`
            // (or `public sealed class Quit : Message { }` for Unity, since classes
            // cannot use the semicolon shorthand)
            quote! {
                if type_kw == "record" {
                    format!("{}public sealed record {} : {}{};", base_indent, #variant_name, #parent_name, generic_suffix)
                } else {
                    format!("{}public sealed class {} : {}{} {{ }}", base_indent, #variant_name, #parent_name, generic_suffix)
                }
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            // Newtype variant: single `Value` property.
            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{base}public sealed {type_kw} {name} : {parent}{generics}\n\
                         {base}{{\n\
                         {prop}[{attr}(\"Value\")]\n\
                         {prop}public {req}{ty} Value {{ get; {acc}; }}\n\
                         {base}}}",
                        base = base_indent,
                        type_kw = type_kw,
                        name = #variant_name,
                        parent = #parent_name,
                        generics = generic_suffix,
                        prop = prop_indent,
                        attr = attr_name,
                        req = required_kw,
                        ty = csharp_type,
                        acc = accessor,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            build_struct_variant(parent_name, variant_name, fields)
        }
    }
}

/// Builds the token stream for a struct variant with named fields.
///
/// References runtime variables `base_indent`, `prop_indent`, `attr_name`,
/// `required_kw`, and `extension_data_type` which must be in scope.
fn build_struct_variant(
    parent_name: &str,
    variant_name: &str,
    fields: &[CSharpField],
) -> TokenStream {
    let field_exprs = build_struct_variant_fields(fields);

    quote! {
        {
            let mut field_parts: Vec<String> = Vec::new();
            #(#field_exprs)*
            let fields_block = field_parts.join("\n");
            format!(
                "{base}public sealed {type_kw} {name} : {parent}{generics}\n\
                 {base}{{\n\
                 {fields}\n\
                 {base}}}",
                base = base_indent,
                type_kw = type_kw,
                name = #variant_name,
                parent = #parent_name,
                generics = generic_suffix,
                fields = fields_block,
            )
        }
    }
}

/// Builds the property declaration token streams for a struct variant's fields.
///
/// References runtime variables `prop_indent`, `attr_name`, `required_kw`,
/// and `extension_data_type` which must be in scope.
///
/// Dispatches on [`FlattenKind`] for each field:
/// - `None`: standard `[JsonPropertyName]` + property declaration
/// - `Struct`: runtime iteration over `csharp_fields()` with property + extension data handling
/// - `HashMap`: `[JsonExtensionData]` dictionary property
fn build_struct_variant_fields(fields: &[CSharpField]) -> Vec<TokenStream> {
    fields
        .iter()
        .map(|f| match &f.flatten {
            FlattenKind::None => {
                let prop_name = &f.csharp_property_name;
                let json_name = &f.json_name;
                let is_optional = f.is_optional;
                let type_expr = &f.type_expr;

                quote! {
                    field_parts.push({
                        let csharp_type = #type_expr;
                        let nullable = if #is_optional { "?" } else { "" };
                        let req = if #is_optional { "" } else { required_kw };
                        format!(
                            "{prop}[{attr}(\"{json}\")]\n\
                             {prop}public {req}{ty}{null} {name} {{ get; {acc}; }}",
                            prop = prop_indent,
                            attr = attr_name,
                            json = #json_name,
                            req = req,
                            ty = csharp_type,
                            null = nullable,
                            name = #prop_name,
                            acc = accessor,
                        )
                    });
                }
            }
            FlattenKind::Struct => {
                let type_expr = &f.type_expr;
                quote! {
                    for field_info in #type_expr {
                        match field_info {
                            csharp_rs::CSharpFieldInfo::Property {
                                property_name,
                                json_name,
                                type_name,
                                is_optional,
                            } => {
                                let nullable = if is_optional { "?" } else { "" };
                                let req = if is_optional { "" } else { required_kw };
                                field_parts.push(format!(
                                    "{prop}[{attr}(\"{json}\")]\n\
                                     {prop}public {req}{ty}{null} {name} {{ get; {acc}; }}",
                                    prop = prop_indent,
                                    attr = attr_name,
                                    json = json_name,
                                    req = req,
                                    ty = type_name,
                                    null = nullable,
                                    name = property_name,
                                    acc = accessor,
                                ));
                            }
                            csharp_rs::CSharpFieldInfo::ExtensionData { .. } => {
                                field_parts.push(format!(
                                    "{prop}[JsonExtensionData]\n\
                                     {prop}public {ext_type}? ExtensionData {{ get; set; }}",
                                    prop = prop_indent,
                                    ext_type = extension_data_type,
                                ));
                            }
                        }
                    }
                }
            }
            FlattenKind::HashMap { .. } => {
                quote! {
                    field_parts.push(format!(
                        "{prop}[JsonExtensionData]\n\
                         {prop}public {ext_type}? ExtensionData {{ get; set; }}",
                        prop = prop_indent,
                        ext_type = extension_data_type,
                    ));
                }
            }
        })
        .collect()
}

/// Builds per-field write expressions for converter struct arms.
///
/// Returns `Vec<TokenStream>` where each item is a statement pushing
/// onto a pre-existing `field_lines: Vec<String>`. Handles all three
/// [`FlattenKind`] variants:
/// - `None`: compile-time property write
/// - `Struct`: runtime iteration over `csharp_fields()`
/// - `HashMap`: skipped (extension data handled by serializer)
///
/// References runtime variable `serialize_call` which must be in scope.
/// The caller must also have `converter_body_indent` in scope for the
/// indent level.
fn build_write_field_exprs(fields: &[CSharpField], var_name: &str) -> Vec<TokenStream> {
    fields
        .iter()
        .filter_map(|f| {
            match &f.flatten {
                FlattenKind::None => {
                    let json = &f.json_name;
                    let prop = &f.csharp_property_name;
                    Some(quote! {
                        field_lines.push(format!(
                            "{body}writer.WritePropertyName(\"{json}\");\n\
                             {body}{call}",
                            body = converter_body_indent,
                            json = #json,
                            call = serialize_call.replace("{var}", #var_name).replace("{prop}", #prop),
                        ));
                    })
                }
                FlattenKind::Struct => {
                    let type_expr = &f.type_expr;
                    Some(quote! {
                        for field_info in #type_expr {
                            match field_info {
                                csharp_rs::CSharpFieldInfo::Property {
                                    property_name,
                                    json_name,
                                    type_name: _,
                                    ..
                                } => {
                                    field_lines.push(format!(
                                        "{body}writer.WritePropertyName(\"{json}\");\n\
                                         {body}{call}",
                                        body = converter_body_indent,
                                        json = json_name,
                                        call = serialize_call
                                            .replace("{var}", #var_name)
                                            .replace("{prop}", &property_name),
                                    ));
                                }
                                csharp_rs::CSharpFieldInfo::ExtensionData { .. } => {}
                            }
                        }
                    })
                }
                // Extension data is handled by the serializer; nothing to write.
                FlattenKind::HashMap { .. } => None,
            }
        })
        .collect()
}

/// Builds per-field read expressions for converter struct arms.
///
/// `read_fmt` is a format template with placeholders `{indent}`, `{name}`,
/// `{json}`, `{ty}`.
///
/// References runtime variable `converter_prop_indent` which must be in
/// scope for the indent level.
///
/// Returns `Vec<TokenStream>` where each item pushes onto `field_lines`.
fn build_read_field_exprs(fields: &[CSharpField], read_fmt: &str) -> Vec<TokenStream> {
    fields
        .iter()
        .filter_map(|f| {
            match &f.flatten {
                FlattenKind::None => {
                    let prop_name = &f.csharp_property_name;
                    let field_json = &f.json_name;
                    let type_expr = &f.type_expr;

                    Some(quote! {
                        {
                            let csharp_type = #type_expr;
                            field_lines.push(format!(
                                #read_fmt,
                                indent = converter_prop_indent,
                                name = #prop_name,
                                json = #field_json,
                                ty = csharp_type,
                            ));
                        }
                    })
                }
                FlattenKind::Struct => {
                    let type_expr = &f.type_expr;
                    Some(quote! {
                        for field_info in #type_expr {
                            match field_info {
                                csharp_rs::CSharpFieldInfo::Property {
                                    property_name,
                                    json_name,
                                    type_name,
                                    ..
                                } => {
                                    field_lines.push(format!(
                                        #read_fmt,
                                        indent = converter_prop_indent,
                                        name = property_name,
                                        json = json_name,
                                        ty = type_name,
                                    ));
                                }
                                csharp_rs::CSharpFieldInfo::ExtensionData { .. } => {}
                            }
                        }
                    })
                }
                // Extension data is handled by the serializer; nothing to read.
                FlattenKind::HashMap { .. } => None,
            }
        })
        .collect()
}

/// Builds the converter block for tagged enums.
///
/// Returns `None` for non-internal tagging modes that will always need a
/// converter. Returns `Some(TokenStream)` for all modes. The returned token
/// stream evaluates to a `String` at runtime.
///
/// For internal tagging, the converter is conditional on
/// `!use_native_polymorphism` (runtime check).
///
/// References runtime variables `base_indent`, `serialize_call`, and
/// `use_native_polymorphism` which must be in scope.
fn build_converter_block(
    csharp_name: &str,
    tagging: &EnumTagging,
    variants: &[TaggedVariant],
) -> TokenStream {
    match tagging {
        EnumTagging::Internal { tag } => {
            let stj_converter = build_internal_stj_converter(csharp_name, tag, variants);
            let newtonsoft_converter =
                build_internal_newtonsoft_converter(csharp_name, tag, variants);

            quote! {
                if use_native_polymorphism {
                    String::new()
                } else {
                    match cfg.serializer() {
                        csharp_rs::Serializer::SystemTextJson => { #stj_converter }
                        csharp_rs::Serializer::Newtonsoft => { #newtonsoft_converter }
                    }
                }
            }
        }
        EnumTagging::External => {
            let stj_converter = build_external_stj_converter(csharp_name, variants);
            let newtonsoft_converter = build_external_newtonsoft_converter(csharp_name, variants);

            quote! {
                match cfg.serializer() {
                    csharp_rs::Serializer::SystemTextJson => { #stj_converter }
                    csharp_rs::Serializer::Newtonsoft => { #newtonsoft_converter }
                }
            }
        }
        EnumTagging::Adjacent { tag, content } => {
            let stj_converter = build_adjacent_stj_converter(csharp_name, tag, content, variants);
            let newtonsoft_converter =
                build_adjacent_newtonsoft_converter(csharp_name, tag, content, variants);

            quote! {
                match cfg.serializer() {
                    csharp_rs::Serializer::SystemTextJson => { #stj_converter }
                    csharp_rs::Serializer::Newtonsoft => { #newtonsoft_converter }
                }
            }
        }
        EnumTagging::Untagged => {
            let stj_converter = build_untagged_stj_converter(csharp_name, variants);
            let newtonsoft_converter = build_untagged_newtonsoft_converter(csharp_name, variants);

            quote! {
                match cfg.serializer() {
                    csharp_rs::Serializer::SystemTextJson => { #stj_converter }
                    csharp_rs::Serializer::Newtonsoft => { #newtonsoft_converter }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal tagging converters
// ---------------------------------------------------------------------------

/// Builds the STJ `JsonConverter<T>` for internally tagged enums.
///
/// Generates a `private sealed class {Name}Converter : JsonConverter<{Name}>`
/// with `Read` and `Write` methods that use `JsonDocument` / `Utf8JsonReader`
/// for deserialization and `Utf8JsonWriter` for serialization.
///
/// References runtime variable `base_indent` which must be in scope.
fn build_internal_stj_converter(
    csharp_name: &str,
    tag: &str,
    variants: &[TaggedVariant],
) -> TokenStream {
    let read_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| {
            build_stj_read_arm(
                v,
                "{indent}{name} = root.GetProperty(\"{json}\").Deserialize<{ty}>(options),",
            )
        })
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_stj_write_arm_internal(v, tag))
        .collect();

    quote! {
        {
            let converter_inner = format!("{}    ", base_indent);
            let converter_inner2 = format!("{}    ", converter_inner);
            let converter_inner3 = format!("{}    ", converter_inner2);
            let converter_prop_indent = format!("{}    ", converter_inner3);
            let converter_body_indent = converter_prop_indent.clone();
            let serialize_call = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => {
                    "JsonSerializer.Serialize(writer, {var}.{prop}, options);"
                }
                csharp_rs::Serializer::Newtonsoft => {
                    "serializer.Serialize(writer, {var}.{prop});"
                }
            };
            let converter_arm_indent = converter_inner3.clone();

            let mut read_parts: Vec<String> = Vec::new();
            #(read_parts.push(#read_arms);)*

            let mut write_parts: Vec<String> = Vec::new();
            #(write_parts.push(#write_arms);)*

            let read_block = read_parts.join("\n");
            let write_block = write_parts.join("\n");

            format!(
                "\n\
                 {base}private sealed class {name}Converter{gen} : JsonConverter<{name}{gen}>\n\
                 {base}{{\n\
                 {i1}public override {name}{gen} Read(\n\
                 {i2}ref Utf8JsonReader reader,\n\
                 {i2}Type typeToConvert,\n\
                 {i2}JsonSerializerOptions options)\n\
                 {i1}{{\n\
                 {i2}using var doc = JsonDocument.ParseValue(ref reader);\n\
                 {i2}var root = doc.RootElement;\n\
                 {i2}var tag = root.GetProperty(\"{tag}\").GetString();\n\
                 \n\
                 {i2}return tag switch\n\
                 {i2}{{\n\
                 {read}\n\
                 {i2}    _ => throw new JsonException($\"Unknown discriminator value: {{tag}}\")\n\
                 {i2}}};\n\
                 {i1}}}\n\
                 \n\
                 {i1}public override void Write(\n\
                 {i2}Utf8JsonWriter writer,\n\
                 {i2}{name}{gen} value,\n\
                 {i2}JsonSerializerOptions options)\n\
                 {i1}{{\n\
                 {i2}writer.WriteStartObject();\n\
                 {i2}switch (value)\n\
                 {i2}{{\n\
                 {write}\n\
                 {i2}}}\n\
                 {i2}writer.WriteEndObject();\n\
                 {i1}}}\n\
                 {base}}}",
                base = base_indent,
                name = #csharp_name,
                gen = generic_suffix,
                i1 = converter_inner,
                i2 = converter_inner2,
                tag = #tag,
                read = read_block,
                write = write_block,
            )
        }
    }
}

/// Builds the Newtonsoft `JsonConverter<T>` for internally tagged enums.
///
/// References runtime variables `base_indent` and `generic_suffix` which must
/// be in scope. For generic types, emits a non-generic `JsonConverter` with
/// `CanConvert` override; for non-generic types, emits `JsonConverter<T>`.
#[expect(
    clippy::too_many_lines,
    reason = "branches for generic vs non-generic Newtonsoft converter"
)]
fn build_internal_newtonsoft_converter(
    csharp_name: &str,
    tag: &str,
    variants: &[TaggedVariant],
) -> TokenStream {
    let read_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| {
            build_newtonsoft_read_arm(
                v,
                "{indent}{name} = obj[\"{json}\"].ToObject<{ty}>(serializer),",
            )
        })
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_newtonsoft_write_arm_internal(v, tag))
        .collect();

    quote! {
        {
            let converter_inner = format!("{}    ", base_indent);
            let converter_inner2 = format!("{}    ", converter_inner);
            let converter_inner3 = format!("{}    ", converter_inner2);
            let converter_prop_indent = format!("{}    ", converter_inner3);
            let converter_body_indent = converter_prop_indent.clone();
            let serialize_call = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => {
                    "JsonSerializer.Serialize(writer, {var}.{prop}, options);"
                }
                csharp_rs::Serializer::Newtonsoft => {
                    "serializer.Serialize(writer, {var}.{prop});"
                }
            };
            let converter_arm_indent = converter_inner3.clone();

            let mut read_parts: Vec<String> = Vec::new();
            #(read_parts.push(#read_arms);)*

            let mut write_parts: Vec<String> = Vec::new();
            #(write_parts.push(#write_arms);)*

            let read_block = read_parts.join("\n");
            let write_block = write_parts.join("\n");

            if generic_suffix.is_empty() {
                format!(
                    "\n\
                     {base}private sealed class {name}Converter : JsonConverter<{name}>\n\
                     {base}{{\n\
                     {i1}public override {name} ReadJson(\n\
                     {i2}JsonReader reader,\n\
                     {i2}Type objectType,\n\
                     {i2}{name} existingValue,\n\
                     {i2}bool hasExistingValue,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}var obj = JObject.Load(reader);\n\
                     {i2}var tag = (string)obj[\"{tag}\"];\n\
                     \n\
                     {i2}return tag switch\n\
                     {i2}{{\n\
                     {read}\n\
                     {i2}    _ => throw new JsonException($\"Unknown discriminator value: {{tag}}\")\n\
                     {i2}}};\n\
                     {i1}}}\n\
                     \n\
                     {i1}public override void WriteJson(\n\
                     {i2}JsonWriter writer,\n\
                     {i2}{name} value,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}writer.WriteStartObject();\n\
                     {i2}switch (value)\n\
                     {i2}{{\n\
                     {write}\n\
                     {i2}}}\n\
                     {i2}writer.WriteEndObject();\n\
                     {i1}}}\n\
                     {base}}}",
                    base = base_indent,
                    name = #csharp_name,
                    i1 = converter_inner,
                    i2 = converter_inner2,
                    tag = #tag,
                    read = read_block,
                    write = write_block,
                )
            } else {
                let open_commas = ",".repeat(generic_suffix.matches(',').count());
                format!(
                    "\n\
                     {base}private sealed class {name}Converter : JsonConverter\n\
                     {base}{{\n\
                     {i1}public override bool CanConvert(Type objectType) =>\n\
                     {i2}objectType.IsGenericType && objectType.GetGenericTypeDefinition() == typeof({name}<{oc}>);\n\
                     \n\
                     {i1}public override object ReadJson(\n\
                     {i2}JsonReader reader,\n\
                     {i2}Type objectType,\n\
                     {i2}object existingValue,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}var obj = JObject.Load(reader);\n\
                     {i2}var tag = (string)obj[\"{tag}\"];\n\
                     \n\
                     {i2}return tag switch\n\
                     {i2}{{\n\
                     {read}\n\
                     {i2}    _ => throw new JsonException($\"Unknown discriminator value: {{tag}}\")\n\
                     {i2}}};\n\
                     {i1}}}\n\
                     \n\
                     {i1}public override void WriteJson(\n\
                     {i2}JsonWriter writer,\n\
                     {i2}object value,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}writer.WriteStartObject();\n\
                     {i2}switch (value)\n\
                     {i2}{{\n\
                     {write}\n\
                     {i2}}}\n\
                     {i2}writer.WriteEndObject();\n\
                     {i1}}}\n\
                     {base}}}",
                    base = base_indent,
                    name = #csharp_name,
                    oc = open_commas,
                    i1 = converter_inner,
                    i2 = converter_inner2,
                    tag = #tag,
                    read = read_block,
                    write = write_block,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Read arm builders (shared across tagging modes)
// ---------------------------------------------------------------------------

/// Builds a single STJ Read switch arm for a variant.
///
/// `read_fmt` is a format template for struct field reads with placeholders
/// `{indent}`, `{name}`, `{json}`, `{ty}`.
///
/// References runtime variables `converter_arm_indent` and
/// `converter_prop_indent` which must be in scope.
fn build_stj_read_arm(variant: &TaggedVariant, read_fmt: &str) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            quote! {
                format!(
                    "{arm}\"{json}\" => new {name}(),",
                    arm = converter_arm_indent,
                    json = #json_name,
                    name = #csharp_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {prop}Value = root.GetProperty(\"Value\").Deserialize<{ty}>(options),\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        prop = converter_prop_indent,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_read_field_exprs(fields, read_fmt);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_str = field_lines.join("\n");
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {fields}\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        fields = fields_str,
                    )
                }
            }
        }
    }
}

/// Builds a single Newtonsoft Read switch arm for a variant.
///
/// References runtime variables `converter_arm_indent` and
/// `converter_prop_indent` which must be in scope.
fn build_newtonsoft_read_arm(variant: &TaggedVariant, read_fmt: &str) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            quote! {
                format!(
                    "{arm}\"{json}\" => new {name}(),",
                    arm = converter_arm_indent,
                    json = #json_name,
                    name = #csharp_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {prop}Value = obj[\"Value\"].ToObject<{ty}>(serializer),\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        prop = converter_prop_indent,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_read_field_exprs(fields, read_fmt);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_str = field_lines.join("\n");
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {fields}\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        fields = fields_str,
                    )
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Write arm builders — internal tagging
// ---------------------------------------------------------------------------

/// Builds a single STJ Write switch arm for a variant (internal tagging).
///
/// References runtime variables `converter_arm_indent`,
/// `converter_body_indent`, and `serialize_call` which must be in scope.
fn build_stj_write_arm_internal(variant: &TaggedVariant, tag: &str) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            quote! {
                format!(
                    "{ci}case {name}:\n\
                     {bi}writer.WriteString(\"{tag}\", \"{json}\");\n\
                     {bi}break;",
                    ci = converter_arm_indent,
                    name = #csharp_name,
                    bi = converter_body_indent,
                    tag = #tag,
                    json = #json_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let _ty = #type_expr;
                    format!(
                        "{ci}case {name} {var}:\n\
                         {bi}writer.WriteString(\"{tag}\", \"{json}\");\n\
                         {bi}writer.WritePropertyName(\"Value\");\n\
                         {bi}JsonSerializer.Serialize(writer, {var}.Value, options);\n\
                         {bi}break;",
                        ci = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = converter_body_indent,
                        tag = #tag,
                        json = #json_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_write_field_exprs(fields, &var_name);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_block = field_lines.join("\n");
                    format!(
                        "{case}case {name} {var}:\n\
                         {body}writer.WriteString(\"{tag}\", \"{json}\");\n\
                         {fields}\n\
                         {body}break;",
                        case = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        body = converter_body_indent,
                        tag = #tag,
                        json = #json_name,
                        fields = fields_block,
                    )
                }
            }
        }
    }
}

/// Builds a single Newtonsoft Write switch arm for a variant (internal tagging).
///
/// References runtime variables `converter_arm_indent`,
/// `converter_body_indent`, and `serialize_call` which must be in scope.
fn build_newtonsoft_write_arm_internal(variant: &TaggedVariant, tag: &str) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            quote! {
                format!(
                    "{ci}case {name}:\n\
                     {bi}writer.WritePropertyName(\"{tag}\");\n\
                     {bi}writer.WriteValue(\"{json}\");\n\
                     {bi}break;",
                    ci = converter_arm_indent,
                    name = #csharp_name,
                    bi = converter_body_indent,
                    tag = #tag,
                    json = #json_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let _ty = #type_expr;
                    format!(
                        "{ci}case {name} {var}:\n\
                         {bi}writer.WritePropertyName(\"{tag}\");\n\
                         {bi}writer.WriteValue(\"{json}\");\n\
                         {bi}writer.WritePropertyName(\"Value\");\n\
                         {bi}serializer.Serialize(writer, {var}.Value);\n\
                         {bi}break;",
                        ci = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = converter_body_indent,
                        tag = #tag,
                        json = #json_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_write_field_exprs(fields, &var_name);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_block = field_lines.join("\n");
                    format!(
                        "{case}case {name} {var}:\n\
                         {body}writer.WritePropertyName(\"{tag}\");\n\
                         {body}writer.WriteValue(\"{json}\");\n\
                         {fields}\n\
                         {body}break;",
                        case = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        body = converter_body_indent,
                        tag = #tag,
                        json = #json_name,
                        fields = fields_block,
                    )
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// External tagging converters
// ---------------------------------------------------------------------------

/// Builds the STJ `JsonConverter<T>` for externally tagged enums.
///
/// External tagging is serde's default: unit variants serialize to raw strings
/// (e.g., `"Quit"`), while data variants serialize to single-property objects
/// (e.g., `{"Text": "hello"}` or `{"Request": {"id": "abc"}}`).
///
/// References runtime variables `base_indent` and `generic_suffix` which must
/// be in scope.
#[expect(
    clippy::too_many_lines,
    reason = "builds complete external STJ converter with generic suffix"
)]
fn build_external_stj_converter(csharp_name: &str, variants: &[TaggedVariant]) -> TokenStream {
    let read_unit_arms: Vec<TokenStream> = variants
        .iter()
        .filter(|v| matches!(v.data, TaggedVariantData::Unit))
        .map(build_external_stj_read_unit_arm)
        .collect();

    let read_object_arms: Vec<TokenStream> = variants
        .iter()
        .filter(|v| !matches!(v.data, TaggedVariantData::Unit))
        .map(|v| {
            build_external_stj_read_object_arm(
                v,
                "{indent}{name} = prop.Value.GetProperty(\"{json}\").Deserialize<{ty}>(options),",
            )
        })
        .collect();

    let write_arms: Vec<TokenStream> = variants.iter().map(build_external_stj_write_arm).collect();

    quote! {
        {
            let converter_inner = format!("{}    ", base_indent);
            let converter_inner2 = format!("{}    ", converter_inner);
            let converter_inner3 = format!("{}    ", converter_inner2);
            let converter_inner4 = format!("{}    ", converter_inner3);
            let converter_inner5 = format!("{}    ", converter_inner4);
            let converter_arm_indent = converter_inner4.clone();
            let converter_prop_indent = converter_inner5.clone();
            let converter_body_indent = converter_inner4.clone();
            let serialize_call = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => {
                    "JsonSerializer.Serialize(writer, {var}.{prop}, options);"
                }
                csharp_rs::Serializer::Newtonsoft => {
                    "serializer.Serialize(writer, {var}.{prop});"
                }
            };

            let mut read_unit_parts: Vec<String> = Vec::new();
            #(read_unit_parts.push(#read_unit_arms);)*

            let mut read_object_parts: Vec<String> = Vec::new();
            #(read_object_parts.push(#read_object_arms);)*

            let mut write_parts: Vec<String> = Vec::new();
            #(write_parts.push(#write_arms);)*

            let read_unit_block = read_unit_parts.join("\n");
            let read_object_block = read_object_parts.join("\n");
            let write_block = write_parts.join("\n");

            format!(
                "\n\
                 {base}private sealed class {name}Converter{gen} : JsonConverter<{name}{gen}>\n\
                 {base}{{\n\
                 {i1}public override {name}{gen} Read(\n\
                 {i2}ref Utf8JsonReader reader,\n\
                 {i2}Type typeToConvert,\n\
                 {i2}JsonSerializerOptions options)\n\
                 {i1}{{\n\
                 {i2}using var doc = JsonDocument.ParseValue(ref reader);\n\
                 {i2}var root = doc.RootElement;\n\
                 \n\
                 {i2}if (root.ValueKind == JsonValueKind.String)\n\
                 {i2}{{\n\
                 {i3}var tag = root.GetString();\n\
                 {i3}return tag switch\n\
                 {i3}{{\n\
                 {read_unit}\n\
                 {i3}    _ => throw new JsonException($\"Unknown unit variant: {{tag}}\")\n\
                 {i3}}};\n\
                 {i2}}}\n\
                 \n\
                 {i2}if (root.ValueKind == JsonValueKind.Object)\n\
                 {i2}{{\n\
                 {i3}var prop = root.EnumerateObject().First();\n\
                 {i3}return prop.Name switch\n\
                 {i3}{{\n\
                 {read_obj}\n\
                 {i3}    _ => throw new JsonException($\"Unknown variant: {{prop.Name}}\")\n\
                 {i3}}};\n\
                 {i2}}}\n\
                 \n\
                 {i2}throw new JsonException($\"Unexpected JSON token: {{root.ValueKind}}\");\n\
                 {i1}}}\n\
                 \n\
                 {i1}public override void Write(\n\
                 {i2}Utf8JsonWriter writer,\n\
                 {i2}{name}{gen} value,\n\
                 {i2}JsonSerializerOptions options)\n\
                 {i1}{{\n\
                 {i2}switch (value)\n\
                 {i2}{{\n\
                 {write}\n\
                 {i2}}}\n\
                 {i1}}}\n\
                 {base}}}",
                base = base_indent,
                name = #csharp_name,
                gen = generic_suffix,
                i1 = converter_inner,
                i2 = converter_inner2,
                i3 = converter_inner3,
                read_unit = read_unit_block,
                read_obj = read_object_block,
                write = write_block,
            )
        }
    }
}

/// Builds a STJ Read switch arm for a unit variant in external tagging.
///
/// References runtime variable `converter_arm_indent` which must be in scope.
fn build_external_stj_read_unit_arm(variant: &TaggedVariant) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    quote! {
        format!(
            "{arm}\"{json}\" => new {name}(),",
            arm = converter_arm_indent,
            json = #json_name,
            name = #csharp_name,
        )
    }
}

/// Builds a STJ Read switch arm for a data variant (newtype or struct) in
/// external tagging.
///
/// References runtime variables `converter_arm_indent` and
/// `converter_prop_indent` which must be in scope.
fn build_external_stj_read_object_arm(variant: &TaggedVariant, read_fmt: &str) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            unreachable!("unit variants are handled separately")
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {prop}Value = prop.Value.Deserialize<{ty}>(options),\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        prop = converter_prop_indent,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_read_field_exprs(fields, read_fmt);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_str = field_lines.join("\n");
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {fields}\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        fields = fields_str,
                    )
                }
            }
        }
    }
}

/// Builds a single STJ Write switch arm for a variant in external tagging.
///
/// Unit variants emit `WriteStringValue("Name")`. Data variants wrap content
/// in `WriteStartObject` / `WritePropertyName` / `WriteEndObject`.
///
/// References runtime variables `converter_arm_indent`,
/// `converter_body_indent`, and `serialize_call` which must be in scope.
fn build_external_stj_write_arm(variant: &TaggedVariant) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            quote! {
                format!(
                    "{ci}case {name}:\n\
                     {bi}writer.WriteStringValue(\"{json}\");\n\
                     {bi}break;",
                    ci = converter_arm_indent,
                    name = #csharp_name,
                    bi = converter_body_indent,
                    json = #json_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let _ty = #type_expr;
                    format!(
                        "{ci}case {name} {var}:\n\
                         {bi}writer.WriteStartObject();\n\
                         {bi}writer.WritePropertyName(\"{json}\");\n\
                         {bi}JsonSerializer.Serialize(writer, {var}.Value, options);\n\
                         {bi}writer.WriteEndObject();\n\
                         {bi}break;",
                        ci = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = converter_body_indent,
                        json = #json_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_write_field_exprs(fields, &var_name);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_block = field_lines.join("\n");
                    format!(
                        "{case}case {name} {var}:\n\
                         {body}writer.WriteStartObject();\n\
                         {body}writer.WritePropertyName(\"{json}\");\n\
                         {body}writer.WriteStartObject();\n\
                         {fields}\n\
                         {body}writer.WriteEndObject();\n\
                         {body}writer.WriteEndObject();\n\
                         {body}break;",
                        case = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        body = converter_body_indent,
                        json = #json_name,
                        fields = fields_block,
                    )
                }
            }
        }
    }
}

/// Builds the Newtonsoft `JsonConverter<T>` for externally tagged enums.
///
/// References runtime variable `base_indent` which must be in scope.
#[expect(
    clippy::too_many_lines,
    reason = "builds complete Newtonsoft converter with read/write arms"
)]
fn build_external_newtonsoft_converter(
    csharp_name: &str,
    variants: &[TaggedVariant],
) -> TokenStream {
    let read_unit_arms: Vec<TokenStream> = variants
        .iter()
        .filter(|v| matches!(v.data, TaggedVariantData::Unit))
        .map(build_external_newtonsoft_read_unit_arm)
        .collect();

    let read_object_arms: Vec<TokenStream> = variants
        .iter()
        .filter(|v| !matches!(v.data, TaggedVariantData::Unit))
        .map(|v| {
            build_external_newtonsoft_read_object_arm(
                v,
                "{indent}{name} = prop.Value[\"{json}\"].ToObject<{ty}>(serializer),",
            )
        })
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(build_external_newtonsoft_write_arm)
        .collect();

    quote! {
        {
            let converter_inner = format!("{}    ", base_indent);
            let converter_inner2 = format!("{}    ", converter_inner);
            let converter_inner3 = format!("{}    ", converter_inner2);
            let converter_inner4 = format!("{}    ", converter_inner3);
            let converter_inner5 = format!("{}    ", converter_inner4);
            let converter_arm_indent = converter_inner4.clone();
            let converter_prop_indent = converter_inner5.clone();
            let converter_body_indent = converter_inner4.clone();
            let serialize_call = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => {
                    "JsonSerializer.Serialize(writer, {var}.{prop}, options);"
                }
                csharp_rs::Serializer::Newtonsoft => {
                    "serializer.Serialize(writer, {var}.{prop});"
                }
            };

            let mut read_unit_parts: Vec<String> = Vec::new();
            #(read_unit_parts.push(#read_unit_arms);)*

            let mut read_object_parts: Vec<String> = Vec::new();
            #(read_object_parts.push(#read_object_arms);)*

            let mut write_parts: Vec<String> = Vec::new();
            #(write_parts.push(#write_arms);)*

            let read_unit_block = read_unit_parts.join("\n");
            let read_object_block = read_object_parts.join("\n");
            let write_block = write_parts.join("\n");

            if generic_suffix.is_empty() {
                format!(
                    "\n\
                     {base}private sealed class {name}Converter : JsonConverter<{name}>\n\
                     {base}{{\n\
                     {i1}public override {name} ReadJson(\n\
                     {i2}JsonReader reader,\n\
                     {i2}Type objectType,\n\
                     {i2}{name} existingValue,\n\
                     {i2}bool hasExistingValue,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}if (reader.TokenType == JsonToken.String)\n\
                     {i2}{{\n\
                     {i3}var tag = (string)JValue.ReadFrom(reader);\n\
                     {i3}return tag switch\n\
                     {i3}{{\n\
                     {read_unit}\n\
                     {i3}    _ => throw new JsonException($\"Unknown unit variant: {{tag}}\")\n\
                     {i3}}};\n\
                     {i2}}}\n\
                     \n\
                     {i2}if (reader.TokenType == JsonToken.StartObject)\n\
                     {i2}{{\n\
                     {i3}var obj = JObject.Load(reader);\n\
                     {i3}var prop = obj.Properties().First();\n\
                     {i3}return prop.Name switch\n\
                     {i3}{{\n\
                     {read_obj}\n\
                     {i3}    _ => throw new JsonException($\"Unknown variant: {{prop.Name}}\")\n\
                     {i3}}};\n\
                     {i2}}}\n\
                     \n\
                     {i2}throw new JsonException($\"Unexpected token: {{reader.TokenType}}\");\n\
                     {i1}}}\n\
                     \n\
                     {i1}public override void WriteJson(\n\
                     {i2}JsonWriter writer,\n\
                     {i2}{name} value,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}switch (value)\n\
                     {i2}{{\n\
                     {write}\n\
                     {i2}}}\n\
                     {i1}}}\n\
                     {base}}}",
                    base = base_indent,
                    name = #csharp_name,
                    i1 = converter_inner,
                    i2 = converter_inner2,
                    i3 = converter_inner3,
                    read_unit = read_unit_block,
                    read_obj = read_object_block,
                    write = write_block,
                )
            } else {
                let open_commas = ",".repeat(generic_suffix.matches(',').count());
                format!(
                    "\n\
                     {base}private sealed class {name}Converter : JsonConverter\n\
                     {base}{{\n\
                     {i1}public override bool CanConvert(Type objectType) =>\n\
                     {i2}objectType.IsGenericType && objectType.GetGenericTypeDefinition() == typeof({name}<{oc}>);\n\
                     \n\
                     {i1}public override object ReadJson(\n\
                     {i2}JsonReader reader,\n\
                     {i2}Type objectType,\n\
                     {i2}object existingValue,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}if (reader.TokenType == JsonToken.String)\n\
                     {i2}{{\n\
                     {i3}var tag = (string)JValue.ReadFrom(reader);\n\
                     {i3}return tag switch\n\
                     {i3}{{\n\
                     {read_unit}\n\
                     {i3}    _ => throw new JsonException($\"Unknown unit variant: {{tag}}\")\n\
                     {i3}}};\n\
                     {i2}}}\n\
                     \n\
                     {i2}if (reader.TokenType == JsonToken.StartObject)\n\
                     {i2}{{\n\
                     {i3}var obj = JObject.Load(reader);\n\
                     {i3}var prop = obj.Properties().First();\n\
                     {i3}return prop.Name switch\n\
                     {i3}{{\n\
                     {read_obj}\n\
                     {i3}    _ => throw new JsonException($\"Unknown variant: {{prop.Name}}\")\n\
                     {i3}}};\n\
                     {i2}}}\n\
                     \n\
                     {i2}throw new JsonException($\"Unexpected token: {{reader.TokenType}}\");\n\
                     {i1}}}\n\
                     \n\
                     {i1}public override void WriteJson(\n\
                     {i2}JsonWriter writer,\n\
                     {i2}object value,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}switch (value)\n\
                     {i2}{{\n\
                     {write}\n\
                     {i2}}}\n\
                     {i1}}}\n\
                     {base}}}",
                    base = base_indent,
                    name = #csharp_name,
                    oc = open_commas,
                    i1 = converter_inner,
                    i2 = converter_inner2,
                    i3 = converter_inner3,
                    read_unit = read_unit_block,
                    read_obj = read_object_block,
                    write = write_block,
                )
            }
        }
    }
}

/// Builds a Newtonsoft Read switch arm for a unit variant in external tagging.
fn build_external_newtonsoft_read_unit_arm(variant: &TaggedVariant) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    quote! {
        format!(
            "{arm}\"{json}\" => new {name}(),",
            arm = converter_arm_indent,
            json = #json_name,
            name = #csharp_name,
        )
    }
}

/// Builds a Newtonsoft Read switch arm for a data variant in external tagging.
fn build_external_newtonsoft_read_object_arm(
    variant: &TaggedVariant,
    read_fmt: &str,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            unreachable!("unit variants are handled separately")
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {prop}Value = prop.Value.ToObject<{ty}>(serializer),\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        prop = converter_prop_indent,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_read_field_exprs(fields, read_fmt);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_str = field_lines.join("\n");
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {fields}\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        fields = fields_str,
                    )
                }
            }
        }
    }
}

/// Builds a single Newtonsoft Write switch arm for a variant in external
/// tagging.
///
/// References runtime variables `converter_arm_indent`,
/// `converter_body_indent`, and `serialize_call` which must be in scope.
fn build_external_newtonsoft_write_arm(variant: &TaggedVariant) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            quote! {
                format!(
                    "{ci}case {name}:\n\
                     {bi}writer.WriteValue(\"{json}\");\n\
                     {bi}break;",
                    ci = converter_arm_indent,
                    name = #csharp_name,
                    bi = converter_body_indent,
                    json = #json_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let _ty = #type_expr;
                    format!(
                        "{ci}case {name} {var}:\n\
                         {bi}writer.WriteStartObject();\n\
                         {bi}writer.WritePropertyName(\"{json}\");\n\
                         {bi}serializer.Serialize(writer, {var}.Value);\n\
                         {bi}writer.WriteEndObject();\n\
                         {bi}break;",
                        ci = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = converter_body_indent,
                        json = #json_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_write_field_exprs(fields, &var_name);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_block = field_lines.join("\n");
                    format!(
                        "{case}case {name} {var}:\n\
                         {body}writer.WriteStartObject();\n\
                         {body}writer.WritePropertyName(\"{json}\");\n\
                         {body}writer.WriteStartObject();\n\
                         {fields}\n\
                         {body}writer.WriteEndObject();\n\
                         {body}writer.WriteEndObject();\n\
                         {body}break;",
                        case = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        body = converter_body_indent,
                        json = #json_name,
                        fields = fields_block,
                    )
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Adjacent tagging converters
// ---------------------------------------------------------------------------

/// Builds the STJ `JsonConverter<T>` for adjacently tagged enums.
///
/// Adjacent tagging uses `#[serde(tag = "...", content = "...")]` where the
/// discriminator and payload are sibling properties in a flat object.
///
/// References runtime variable `base_indent` which must be in scope.
fn build_adjacent_stj_converter(
    csharp_name: &str,
    tag: &str,
    content: &str,
    variants: &[TaggedVariant],
) -> TokenStream {
    let read_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_adjacent_stj_read_arm(v, content))
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_adjacent_stj_write_arm(v, tag, content))
        .collect();

    quote! {
        {
            let converter_inner = format!("{}    ", base_indent);
            let converter_inner2 = format!("{}    ", converter_inner);
            let converter_inner3 = format!("{}    ", converter_inner2);
            let converter_arm_indent = converter_inner3.clone();
            let converter_prop_indent = format!("{}    ", converter_inner3);
            let converter_body_indent = converter_prop_indent.clone();
            let serialize_call = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => {
                    "JsonSerializer.Serialize(writer, {var}.{prop}, options);"
                }
                csharp_rs::Serializer::Newtonsoft => {
                    "serializer.Serialize(writer, {var}.{prop});"
                }
            };

            let mut read_parts: Vec<String> = Vec::new();
            #(read_parts.push(#read_arms);)*

            let mut write_parts: Vec<String> = Vec::new();
            #(write_parts.push(#write_arms);)*

            let read_block = read_parts.join("\n");
            let write_block = write_parts.join("\n");

            format!(
                "\n\
                 {base}private sealed class {name}Converter{gen} : JsonConverter<{name}{gen}>\n\
                 {base}{{\n\
                 {i1}public override {name}{gen} Read(\n\
                 {i2}ref Utf8JsonReader reader,\n\
                 {i2}Type typeToConvert,\n\
                 {i2}JsonSerializerOptions options)\n\
                 {i1}{{\n\
                 {i2}using var doc = JsonDocument.ParseValue(ref reader);\n\
                 {i2}var root = doc.RootElement;\n\
                 {i2}var tag = root.GetProperty(\"{tag}\").GetString();\n\
                 \n\
                 {i2}return tag switch\n\
                 {i2}{{\n\
                 {read}\n\
                 {i2}    _ => throw new JsonException($\"Unknown discriminator value: {{tag}}\")\n\
                 {i2}}};\n\
                 {i1}}}\n\
                 \n\
                 {i1}public override void Write(\n\
                 {i2}Utf8JsonWriter writer,\n\
                 {i2}{name}{gen} value,\n\
                 {i2}JsonSerializerOptions options)\n\
                 {i1}{{\n\
                 {i2}switch (value)\n\
                 {i2}{{\n\
                 {write}\n\
                 {i2}}}\n\
                 {i1}}}\n\
                 {base}}}",
                base = base_indent,
                name = #csharp_name,
                gen = generic_suffix,
                i1 = converter_inner,
                i2 = converter_inner2,
                tag = #tag,
                read = read_block,
                write = write_block,
            )
        }
    }
}

/// Builds a single STJ Read switch arm for a variant in adjacent tagging.
///
/// References runtime variables `converter_arm_indent` and
/// `converter_prop_indent` which must be in scope.
fn build_adjacent_stj_read_arm(variant: &TaggedVariant, content: &str) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            quote! {
                format!(
                    "{arm}\"{json}\" => new {name}(),",
                    arm = converter_arm_indent,
                    json = #json_name,
                    name = #csharp_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {prop}Value = root.GetProperty(\"{content}\").Deserialize<{ty}>(options),\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        prop = converter_prop_indent,
                        content = #content,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let read_fmt = format!(
                "{{indent}}{{name}} = root.GetProperty(\"{content}\").GetProperty(\"{{json}}\").Deserialize<{{ty}}>(options),"
            );
            let field_exprs = build_read_field_exprs(fields, &read_fmt);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_str = field_lines.join("\n");
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {fields}\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        fields = fields_str,
                    )
                }
            }
        }
    }
}

/// Builds a single STJ Write switch arm for a variant in adjacent tagging.
///
/// References runtime variables `converter_arm_indent`,
/// `converter_body_indent`, and `serialize_call` which must be in scope.
fn build_adjacent_stj_write_arm(variant: &TaggedVariant, tag: &str, content: &str) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            // Unit: write start object, tag string, end object (no content key).
            quote! {
                format!(
                    "{ci}case {name}:\n\
                     {bi}writer.WriteStartObject();\n\
                     {bi}writer.WriteString(\"{tag}\", \"{json}\");\n\
                     {bi}writer.WriteEndObject();\n\
                     {bi}break;",
                    ci = converter_arm_indent,
                    name = #csharp_name,
                    bi = converter_body_indent,
                    tag = #tag,
                    json = #json_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let _ty = #type_expr;
                    format!(
                        "{ci}case {name} {var}:\n\
                         {bi}writer.WriteStartObject();\n\
                         {bi}writer.WriteString(\"{tag}\", \"{json}\");\n\
                         {bi}writer.WritePropertyName(\"{content}\");\n\
                         {bi}JsonSerializer.Serialize(writer, {var}.Value, options);\n\
                         {bi}writer.WriteEndObject();\n\
                         {bi}break;",
                        ci = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = converter_body_indent,
                        tag = #tag,
                        json = #json_name,
                        content = #content,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_write_field_exprs(fields, &var_name);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_block = field_lines.join("\n");
                    format!(
                        "{case}case {name} {var}:\n\
                         {body}writer.WriteStartObject();\n\
                         {body}writer.WriteString(\"{tag}\", \"{json}\");\n\
                         {body}writer.WritePropertyName(\"{content}\");\n\
                         {body}writer.WriteStartObject();\n\
                         {fields}\n\
                         {body}writer.WriteEndObject();\n\
                         {body}writer.WriteEndObject();\n\
                         {body}break;",
                        case = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        body = converter_body_indent,
                        tag = #tag,
                        json = #json_name,
                        content = #content,
                        fields = fields_block,
                    )
                }
            }
        }
    }
}

/// Builds the Newtonsoft `JsonConverter<T>` for adjacently tagged enums.
///
/// References runtime variables `base_indent` and `generic_suffix` which must
/// be in scope.
#[expect(
    clippy::too_many_lines,
    reason = "branches for generic vs non-generic Newtonsoft converter"
)]
fn build_adjacent_newtonsoft_converter(
    csharp_name: &str,
    tag: &str,
    content: &str,
    variants: &[TaggedVariant],
) -> TokenStream {
    let read_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_adjacent_newtonsoft_read_arm(v, content))
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_adjacent_newtonsoft_write_arm(v, tag, content))
        .collect();

    quote! {
        {
            let converter_inner = format!("{}    ", base_indent);
            let converter_inner2 = format!("{}    ", converter_inner);
            let converter_inner3 = format!("{}    ", converter_inner2);
            let converter_arm_indent = converter_inner3.clone();
            let converter_prop_indent = format!("{}    ", converter_inner3);
            let converter_body_indent = converter_prop_indent.clone();
            let serialize_call = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => {
                    "JsonSerializer.Serialize(writer, {var}.{prop}, options);"
                }
                csharp_rs::Serializer::Newtonsoft => {
                    "serializer.Serialize(writer, {var}.{prop});"
                }
            };

            let mut read_parts: Vec<String> = Vec::new();
            #(read_parts.push(#read_arms);)*

            let mut write_parts: Vec<String> = Vec::new();
            #(write_parts.push(#write_arms);)*

            let read_block = read_parts.join("\n");
            let write_block = write_parts.join("\n");

            if generic_suffix.is_empty() {
                format!(
                    "\n\
                     {base}private sealed class {name}Converter : JsonConverter<{name}>\n\
                     {base}{{\n\
                     {i1}public override {name} ReadJson(\n\
                     {i2}JsonReader reader,\n\
                     {i2}Type objectType,\n\
                     {i2}{name} existingValue,\n\
                     {i2}bool hasExistingValue,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}var obj = JObject.Load(reader);\n\
                     {i2}var tag = (string)obj[\"{tag}\"];\n\
                     \n\
                     {i2}return tag switch\n\
                     {i2}{{\n\
                     {read}\n\
                     {i2}    _ => throw new JsonException($\"Unknown discriminator value: {{tag}}\")\n\
                     {i2}}};\n\
                     {i1}}}\n\
                     \n\
                     {i1}public override void WriteJson(\n\
                     {i2}JsonWriter writer,\n\
                     {i2}{name} value,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}switch (value)\n\
                     {i2}{{\n\
                     {write}\n\
                     {i2}}}\n\
                     {i1}}}\n\
                     {base}}}",
                    base = base_indent,
                    name = #csharp_name,
                    i1 = converter_inner,
                    i2 = converter_inner2,
                    tag = #tag,
                    read = read_block,
                    write = write_block,
                )
            } else {
                let open_commas = ",".repeat(generic_suffix.matches(',').count());
                format!(
                    "\n\
                     {base}private sealed class {name}Converter : JsonConverter\n\
                     {base}{{\n\
                     {i1}public override bool CanConvert(Type objectType) =>\n\
                     {i2}objectType.IsGenericType && objectType.GetGenericTypeDefinition() == typeof({name}<{oc}>);\n\
                     \n\
                     {i1}public override object ReadJson(\n\
                     {i2}JsonReader reader,\n\
                     {i2}Type objectType,\n\
                     {i2}object existingValue,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}var obj = JObject.Load(reader);\n\
                     {i2}var tag = (string)obj[\"{tag}\"];\n\
                     \n\
                     {i2}return tag switch\n\
                     {i2}{{\n\
                     {read}\n\
                     {i2}    _ => throw new JsonException($\"Unknown discriminator value: {{tag}}\")\n\
                     {i2}}};\n\
                     {i1}}}\n\
                     \n\
                     {i1}public override void WriteJson(\n\
                     {i2}JsonWriter writer,\n\
                     {i2}object value,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}switch (value)\n\
                     {i2}{{\n\
                     {write}\n\
                     {i2}}}\n\
                     {i1}}}\n\
                     {base}}}",
                    base = base_indent,
                    name = #csharp_name,
                    oc = open_commas,
                    i1 = converter_inner,
                    i2 = converter_inner2,
                    tag = #tag,
                    read = read_block,
                    write = write_block,
                )
            }
        }
    }
}

/// Builds a single Newtonsoft Read switch arm for a variant in adjacent
/// tagging.
///
/// References runtime variables `converter_arm_indent` and
/// `converter_prop_indent` which must be in scope.
fn build_adjacent_newtonsoft_read_arm(variant: &TaggedVariant, content: &str) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            quote! {
                format!(
                    "{arm}\"{json}\" => new {name}(),",
                    arm = converter_arm_indent,
                    json = #json_name,
                    name = #csharp_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {prop}Value = obj[\"{content}\"].ToObject<{ty}>(serializer),\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        prop = converter_prop_indent,
                        content = #content,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let read_fmt = format!(
                "{{indent}}{{name}} = obj[\"{content}\"][\"{{json}}\"].ToObject<{{ty}}>(serializer),"
            );
            let field_exprs = build_read_field_exprs(fields, &read_fmt);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_str = field_lines.join("\n");
                    format!(
                        "{arm}\"{json}\" => new {name}\n\
                         {arm}{{\n\
                         {fields}\n\
                         {arm}}},",
                        arm = converter_arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        fields = fields_str,
                    )
                }
            }
        }
    }
}

/// Builds a single Newtonsoft Write switch arm for a variant in adjacent
/// tagging.
///
/// References runtime variables `converter_arm_indent`,
/// `converter_body_indent`, and `serialize_call` which must be in scope.
fn build_adjacent_newtonsoft_write_arm(
    variant: &TaggedVariant,
    tag: &str,
    content: &str,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            // Unit: write start object, tag property + value, end object.
            quote! {
                format!(
                    "{ci}case {name}:\n\
                     {bi}writer.WriteStartObject();\n\
                     {bi}writer.WritePropertyName(\"{tag}\");\n\
                     {bi}writer.WriteValue(\"{json}\");\n\
                     {bi}writer.WriteEndObject();\n\
                     {bi}break;",
                    ci = converter_arm_indent,
                    name = #csharp_name,
                    bi = converter_body_indent,
                    tag = #tag,
                    json = #json_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let _ty = #type_expr;
                    format!(
                        "{ci}case {name} {var}:\n\
                         {bi}writer.WriteStartObject();\n\
                         {bi}writer.WritePropertyName(\"{tag}\");\n\
                         {bi}writer.WriteValue(\"{json}\");\n\
                         {bi}writer.WritePropertyName(\"{content}\");\n\
                         {bi}serializer.Serialize(writer, {var}.Value);\n\
                         {bi}writer.WriteEndObject();\n\
                         {bi}break;",
                        ci = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = converter_body_indent,
                        tag = #tag,
                        json = #json_name,
                        content = #content,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_write_field_exprs(fields, &var_name);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_block = field_lines.join("\n");
                    format!(
                        "{case}case {name} {var}:\n\
                         {body}writer.WriteStartObject();\n\
                         {body}writer.WritePropertyName(\"{tag}\");\n\
                         {body}writer.WriteValue(\"{json}\");\n\
                         {body}writer.WritePropertyName(\"{content}\");\n\
                         {body}writer.WriteStartObject();\n\
                         {fields}\n\
                         {body}writer.WriteEndObject();\n\
                         {body}writer.WriteEndObject();\n\
                         {body}break;",
                        case = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        body = converter_body_indent,
                        tag = #tag,
                        json = #json_name,
                        content = #content,
                        fields = fields_block,
                    )
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Untagged converters
// ---------------------------------------------------------------------------

/// Builds the STJ `JsonConverter<T>` for untagged enums.
///
/// Untagged enums have no discriminator; deserialization tries each variant in
/// declaration order using try/catch.
///
/// References runtime variable `base_indent` which must be in scope.
fn build_untagged_stj_converter(csharp_name: &str, variants: &[TaggedVariant]) -> TokenStream {
    let read_attempts: Vec<TokenStream> = variants
        .iter()
        .map(build_untagged_stj_read_attempt)
        .collect();

    let write_arms: Vec<TokenStream> = variants.iter().map(build_untagged_stj_write_arm).collect();

    quote! {
        {
            let converter_inner = format!("{}    ", base_indent);
            let converter_inner2 = format!("{}    ", converter_inner);
            let converter_inner3 = format!("{}    ", converter_inner2);
            let converter_arm_indent = converter_inner3.clone();
            let converter_prop_indent = format!("{}    ", converter_inner3);
            let converter_body_indent = converter_prop_indent.clone();
            let serialize_call = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => {
                    "JsonSerializer.Serialize(writer, {var}.{prop}, options);"
                }
                csharp_rs::Serializer::Newtonsoft => {
                    "serializer.Serialize(writer, {var}.{prop});"
                }
            };

            let mut read_parts: Vec<String> = Vec::new();
            #(read_parts.push(#read_attempts);)*

            let mut write_parts: Vec<String> = Vec::new();
            #(write_parts.push(#write_arms);)*

            let read_block = read_parts.join("\n");
            let write_block = write_parts.join("\n");

            format!(
                "\n\
                 {base}private sealed class {name}Converter{gen} : JsonConverter<{name}{gen}>\n\
                 {base}{{\n\
                 {i1}public override {name}{gen} Read(\n\
                 {i2}ref Utf8JsonReader reader,\n\
                 {i2}Type typeToConvert,\n\
                 {i2}JsonSerializerOptions options)\n\
                 {i1}{{\n\
                 {i2}using var doc = JsonDocument.ParseValue(ref reader);\n\
                 {i2}var root = doc.RootElement;\n\
                 \n\
                 {read}\n\
                 \n\
                 {i2}throw new JsonException(\"No matching variant found for {name}\");\n\
                 {i1}}}\n\
                 \n\
                 {i1}public override void Write(\n\
                 {i2}Utf8JsonWriter writer,\n\
                 {i2}{name}{gen} value,\n\
                 {i2}JsonSerializerOptions options)\n\
                 {i1}{{\n\
                 {i2}switch (value)\n\
                 {i2}{{\n\
                 {write}\n\
                 {i2}}}\n\
                 {i1}}}\n\
                 {base}}}",
                base = base_indent,
                name = #csharp_name,
                gen = generic_suffix,
                i1 = converter_inner,
                i2 = converter_inner2,
                read = read_block,
                write = write_block,
            )
        }
    }
}

/// Builds a single STJ Read attempt block for a variant in untagged mode.
///
/// References runtime variables `converter_inner2` and `converter_inner3`
/// which must be in scope.
fn build_untagged_stj_read_attempt(variant: &TaggedVariant) -> TokenStream {
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            // Unit variants: check for null.
            quote! {
                format!(
                    "{ai}if (root.ValueKind == JsonValueKind.Null)\n\
                     {ai}{{\n\
                     {bi}return new {name}();\n\
                     {ai}}}",
                    ai = converter_inner2,
                    bi = converter_inner3,
                    name = #csharp_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{ai}try\n\
                         {ai}{{\n\
                         {bi}var val = root.Deserialize<{ty}>(options);\n\
                         {bi}return new {name} {{ Value = val }};\n\
                         {ai}}}\n\
                         {ai}catch (Exception) {{ }}",
                        ai = converter_inner2,
                        bi = converter_inner3,
                        ty = csharp_type,
                        name = #csharp_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let read_fmt =
                "{indent}    {name} = root.GetProperty(\"{json}\").Deserialize<{ty}>(options),";
            let field_exprs = build_read_field_exprs(fields, read_fmt);

            quote! {
                {
                    let converter_prop_indent = converter_inner3.clone();
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_str = field_lines.join("\n");
                    format!(
                        "{ai}try\n\
                         {ai}{{\n\
                         {bi}return new {name}\n\
                         {bi}{{\n\
                         {fields}\n\
                         {bi}}};\n\
                         {ai}}}\n\
                         {ai}catch (Exception) {{ }}",
                        ai = converter_inner2,
                        bi = converter_inner3,
                        name = #csharp_name,
                        fields = fields_str,
                    )
                }
            }
        }
    }
}

/// Builds a single STJ Write switch arm for a variant in untagged mode.
///
/// References runtime variables `converter_arm_indent`,
/// `converter_body_indent`, and `serialize_call` which must be in scope.
fn build_untagged_stj_write_arm(variant: &TaggedVariant) -> TokenStream {
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            quote! {
                format!(
                    "{ci}case {name}:\n\
                     {bi}writer.WriteNullValue();\n\
                     {bi}break;",
                    ci = converter_arm_indent,
                    name = #csharp_name,
                    bi = converter_body_indent,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let _ty = #type_expr;
                    format!(
                        "{ci}case {name} {var}:\n\
                         {bi}JsonSerializer.Serialize(writer, {var}.Value, options);\n\
                         {bi}break;",
                        ci = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = converter_body_indent,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_write_field_exprs(fields, &var_name);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_block = field_lines.join("\n");
                    format!(
                        "{case}case {name} {var}:\n\
                         {body}writer.WriteStartObject();\n\
                         {fields}\n\
                         {body}writer.WriteEndObject();\n\
                         {body}break;",
                        case = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        body = converter_body_indent,
                        fields = fields_block,
                    )
                }
            }
        }
    }
}

/// Builds the Newtonsoft `JsonConverter<T>` for untagged enums.
///
/// References runtime variables `base_indent` and `generic_suffix` which must
/// be in scope.
#[expect(
    clippy::too_many_lines,
    reason = "branches for generic vs non-generic Newtonsoft converter"
)]
fn build_untagged_newtonsoft_converter(
    csharp_name: &str,
    variants: &[TaggedVariant],
) -> TokenStream {
    let read_attempts: Vec<TokenStream> = variants
        .iter()
        .map(build_untagged_newtonsoft_read_attempt)
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(build_untagged_newtonsoft_write_arm)
        .collect();

    quote! {
        {
            let converter_inner = format!("{}    ", base_indent);
            let converter_inner2 = format!("{}    ", converter_inner);
            let converter_inner3 = format!("{}    ", converter_inner2);
            let converter_arm_indent = converter_inner3.clone();
            let converter_prop_indent = format!("{}    ", converter_inner3);
            let converter_body_indent = converter_prop_indent.clone();
            let serialize_call = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => {
                    "JsonSerializer.Serialize(writer, {var}.{prop}, options);"
                }
                csharp_rs::Serializer::Newtonsoft => {
                    "serializer.Serialize(writer, {var}.{prop});"
                }
            };

            let mut read_parts: Vec<String> = Vec::new();
            #(read_parts.push(#read_attempts);)*

            let mut write_parts: Vec<String> = Vec::new();
            #(write_parts.push(#write_arms);)*

            let read_block = read_parts.join("\n");
            let write_block = write_parts.join("\n");

            if generic_suffix.is_empty() {
                format!(
                    "\n\
                     {base}private sealed class {name}Converter : JsonConverter<{name}>\n\
                     {base}{{\n\
                     {i1}public override {name} ReadJson(\n\
                     {i2}JsonReader reader,\n\
                     {i2}Type objectType,\n\
                     {i2}{name} existingValue,\n\
                     {i2}bool hasExistingValue,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}var token = JToken.ReadFrom(reader);\n\
                     \n\
                     {read}\n\
                     \n\
                     {i2}throw new JsonException(\"No matching variant found for {name}\");\n\
                     {i1}}}\n\
                     \n\
                     {i1}public override void WriteJson(\n\
                     {i2}JsonWriter writer,\n\
                     {i2}{name} value,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}switch (value)\n\
                     {i2}{{\n\
                     {write}\n\
                     {i2}}}\n\
                     {i1}}}\n\
                     {base}}}",
                    base = base_indent,
                    name = #csharp_name,
                    i1 = converter_inner,
                    i2 = converter_inner2,
                    read = read_block,
                    write = write_block,
                )
            } else {
                let open_commas = ",".repeat(generic_suffix.matches(',').count());
                format!(
                    "\n\
                     {base}private sealed class {name}Converter : JsonConverter\n\
                     {base}{{\n\
                     {i1}public override bool CanConvert(Type objectType) =>\n\
                     {i2}objectType.IsGenericType && objectType.GetGenericTypeDefinition() == typeof({name}<{oc}>);\n\
                     \n\
                     {i1}public override object ReadJson(\n\
                     {i2}JsonReader reader,\n\
                     {i2}Type objectType,\n\
                     {i2}object existingValue,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}var token = JToken.ReadFrom(reader);\n\
                     \n\
                     {read}\n\
                     \n\
                     {i2}throw new JsonException(\"No matching variant found for {name}\");\n\
                     {i1}}}\n\
                     \n\
                     {i1}public override void WriteJson(\n\
                     {i2}JsonWriter writer,\n\
                     {i2}object value,\n\
                     {i2}JsonSerializer serializer)\n\
                     {i1}{{\n\
                     {i2}switch (value)\n\
                     {i2}{{\n\
                     {write}\n\
                     {i2}}}\n\
                     {i1}}}\n\
                     {base}}}",
                    base = base_indent,
                    name = #csharp_name,
                    oc = open_commas,
                    i1 = converter_inner,
                    i2 = converter_inner2,
                    read = read_block,
                    write = write_block,
                )
            }
        }
    }
}

/// Builds a single Newtonsoft Read attempt block for a variant in untagged
/// mode.
///
/// References runtime variables `converter_inner2` and `converter_inner3`
/// which must be in scope.
fn build_untagged_newtonsoft_read_attempt(variant: &TaggedVariant) -> TokenStream {
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            // Unit variants: check for null token.
            quote! {
                format!(
                    "{ai}if (token.Type == JTokenType.Null)\n\
                     {ai}{{\n\
                     {bi}return new {name}();\n\
                     {ai}}}",
                    ai = converter_inner2,
                    bi = converter_inner3,
                    name = #csharp_name,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{ai}try\n\
                         {ai}{{\n\
                         {bi}var val = token.ToObject<{ty}>(serializer);\n\
                         {bi}return new {name} {{ Value = val }};\n\
                         {ai}}}\n\
                         {ai}catch (Exception) {{ }}",
                        ai = converter_inner2,
                        bi = converter_inner3,
                        ty = csharp_type,
                        name = #csharp_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let read_fmt = "{indent}    {name} = obj[\"{json}\"].ToObject<{ty}>(serializer),";
            let field_exprs = build_read_field_exprs(fields, read_fmt);

            quote! {
                {
                    let converter_prop_indent = converter_inner3.clone();
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_str = field_lines.join("\n");
                    format!(
                        "{ai}try\n\
                         {ai}{{\n\
                         {bi}var obj = (JObject)token;\n\
                         {bi}return new {name}\n\
                         {bi}{{\n\
                         {fields}\n\
                         {bi}}};\n\
                         {ai}}}\n\
                         {ai}catch (Exception) {{ }}",
                        ai = converter_inner2,
                        bi = converter_inner3,
                        name = #csharp_name,
                        fields = fields_str,
                    )
                }
            }
        }
    }
}

/// Builds a single Newtonsoft Write switch arm for a variant in untagged mode.
///
/// References runtime variables `converter_arm_indent`,
/// `converter_body_indent`, and `serialize_call` which must be in scope.
fn build_untagged_newtonsoft_write_arm(variant: &TaggedVariant) -> TokenStream {
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            quote! {
                format!(
                    "{ci}case {name}:\n\
                     {bi}writer.WriteNull();\n\
                     {bi}break;",
                    ci = converter_arm_indent,
                    name = #csharp_name,
                    bi = converter_body_indent,
                )
            }
        }
        TaggedVariantData::Newtype { type_expr } => {
            quote! {
                {
                    let _ty = #type_expr;
                    format!(
                        "{ci}case {name} {var}:\n\
                         {bi}serializer.Serialize(writer, {var}.Value);\n\
                         {bi}break;",
                        ci = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = converter_body_indent,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            let field_exprs = build_write_field_exprs(fields, &var_name);

            quote! {
                {
                    let mut field_lines: Vec<String> = Vec::new();
                    #(#field_exprs)*
                    let fields_block = field_lines.join("\n");
                    format!(
                        "{case}case {name} {var}:\n\
                         {body}writer.WriteStartObject();\n\
                         {fields}\n\
                         {body}writer.WriteEndObject();\n\
                         {body}break;",
                        case = converter_arm_indent,
                        name = #csharp_name,
                        var = #var_name,
                        body = converter_body_indent,
                        fields = fields_block,
                    )
                }
            }
        }
    }
}
