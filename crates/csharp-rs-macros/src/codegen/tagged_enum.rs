// Rust guideline compliant 2026-02-11
//! Tagged enum code generation (internally, adjacently, externally tagged,
//! and untagged).
//!
//! Generates C# abstract record type hierarchies with sealed derived records.
//! Dispatches to the appropriate code generation strategy based on the
//! [`EnumTagging`] variant and the target C# version / serializer combination.
//!
//! When native polymorphism is not available (STJ C# 9-10, or Newtonsoft at
//! any version), a custom `JsonConverter<T>` is generated as a nested
//! `private sealed class` inside the abstract record.

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
/// attributes, nested sealed record declarations, and optionally a custom
/// `JsonConverter<T>` class when native polymorphism is not available.
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

    let converter_expr =
        build_converter_block(csharp_name, tagging, variants, config, use_file_scoped_ns);

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
            converter_expr.as_ref(),
        )
    } else {
        build_block_scoped(
            &using_block,
            namespace,
            &class_attrs,
            csharp_name,
            indent,
            &variant_exprs,
            converter_expr.as_ref(),
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

/// Builds the optional converter block for tagged enums.
///
/// Returns `None` when native polymorphism is used (STJ + C# 11+, internal
/// tagging). Otherwise returns a token stream expression that evaluates to
/// the converter class body as a `String` at runtime.
fn build_converter_block(
    csharp_name: &str,
    tagging: &EnumTagging,
    variants: &[TaggedVariant],
    config: &CSharpConfig,
    use_file_scoped_ns: bool,
) -> Option<TokenStream> {
    let use_native_polymorphism = matches!(tagging, EnumTagging::Internal { .. })
        && config.serializer == Serializer::SystemTextJson
        && config.target >= CSharpVersion::CSharp11;

    if use_native_polymorphism {
        return None;
    }

    match tagging {
        EnumTagging::Internal { tag } => {
            let base_indent = if use_file_scoped_ns {
                "    "
            } else {
                "        "
            };

            match config.serializer {
                Serializer::SystemTextJson => Some(build_internal_stj_converter(
                    csharp_name,
                    tag,
                    variants,
                    base_indent,
                )),
                Serializer::Newtonsoft => Some(build_internal_newtonsoft_converter(
                    csharp_name,
                    tag,
                    variants,
                    base_indent,
                )),
            }
        }
        EnumTagging::External | EnumTagging::Adjacent { .. } | EnumTagging::Untagged => {
            todo!("converter generation for non-internal tagging modes")
        }
    }
}

/// Builds the STJ `JsonConverter<T>` for internally tagged enums.
///
/// Generates a `private sealed class {Name}Converter : JsonConverter<{Name}>`
/// with `Read` and `Write` methods that use `JsonDocument` / `Utf8JsonReader`
/// for deserialization and `Utf8JsonWriter` for serialization.
fn build_internal_stj_converter(
    csharp_name: &str,
    tag: &str,
    variants: &[TaggedVariant],
    base_indent: &str,
) -> TokenStream {
    let inner = format!("{base_indent}    ");
    let inner2 = format!("{inner}    ");
    let inner3 = format!("{inner2}    ");
    let inner4 = format!("{inner3}    ");

    let read_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_stj_read_arm(v, &inner3, &inner4))
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_stj_write_arm(v, tag, &inner3, &inner4))
        .collect();

    quote! {
        {
            let mut read_parts: Vec<String> = Vec::new();
            #(read_parts.push(#read_arms);)*

            let mut write_parts: Vec<String> = Vec::new();
            #(write_parts.push(#write_arms);)*

            let read_block = read_parts.join("\n");
            let write_block = write_parts.join("\n");

            format!(
                "\n\
                 {base}private sealed class {name}Converter : JsonConverter<{name}>\n\
                 {base}{{\n\
                 {i1}public override {name} Read(\n\
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
                 {i2}{name} value,\n\
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
                base = #base_indent,
                name = #csharp_name,
                i1 = #inner,
                i2 = #inner2,
                tag = #tag,
                read = read_block,
                write = write_block,
            )
        }
    }
}

/// Builds the Newtonsoft `JsonConverter<T>` for internally tagged enums.
///
/// Generates a `private sealed class {Name}Converter : JsonConverter<{Name}>`
/// with `ReadJson` and `WriteJson` methods that use `JObject` for
/// deserialization and `JsonWriter` for serialization.
fn build_internal_newtonsoft_converter(
    csharp_name: &str,
    tag: &str,
    variants: &[TaggedVariant],
    base_indent: &str,
) -> TokenStream {
    let inner = format!("{base_indent}    ");
    let inner2 = format!("{inner}    ");
    let inner3 = format!("{inner2}    ");
    let inner4 = format!("{inner3}    ");

    let read_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_newtonsoft_read_arm(v, &inner3, &inner4))
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_newtonsoft_write_arm(v, tag, &inner3, &inner4))
        .collect();

    quote! {
        {
            let mut read_parts: Vec<String> = Vec::new();
            #(read_parts.push(#read_arms);)*

            let mut write_parts: Vec<String> = Vec::new();
            #(write_parts.push(#write_arms);)*

            let read_block = read_parts.join("\n");
            let write_block = write_parts.join("\n");

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
                base = #base_indent,
                name = #csharp_name,
                i1 = #inner,
                i2 = #inner2,
                tag = #tag,
                read = read_block,
                write = write_block,
            )
        }
    }
}

/// Builds a single STJ Read switch arm for a variant.
///
/// Returns a token stream that evaluates to the arm string at runtime
/// (runtime evaluation is needed for type names from `type_expr`).
fn build_stj_read_arm(variant: &TaggedVariant, arm_indent: &str, prop_indent: &str) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            let arm = format!("{arm_indent}\"{json_name}\" => new {csharp_name}(),");
            quote! { String::from(#arm) }
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
                        arm = #arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        prop = #prop_indent,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            build_stj_read_struct_arm(csharp_name, json_name, fields, arm_indent, prop_indent)
        }
    }
}

/// Builds a STJ Read switch arm for a struct variant with named fields.
fn build_stj_read_struct_arm(
    csharp_name: &str,
    json_name: &str,
    fields: &[CSharpField],
    arm_indent: &str,
    prop_indent: &str,
) -> TokenStream {
    let field_exprs: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let prop_name = &f.csharp_property_name;
            let field_json = &f.json_name;
            let type_expr = &f.type_expr;

            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{prop}{name} = root.GetProperty(\"{json}\").Deserialize<{ty}>(options),",
                        prop = #prop_indent,
                        name = #prop_name,
                        json = #field_json,
                        ty = csharp_type,
                    )
                }
            }
        })
        .collect();

    quote! {
        {
            let field_lines: Vec<String> = vec![#(#field_exprs),*];
            let fields_str = field_lines.join("\n");
            format!(
                "{arm}\"{json}\" => new {name}\n\
                 {arm}{{\n\
                 {fields}\n\
                 {arm}}},",
                arm = #arm_indent,
                json = #json_name,
                name = #csharp_name,
                fields = fields_str,
            )
        }
    }
}

/// Builds a single STJ Write switch arm for a variant.
fn build_stj_write_arm(
    variant: &TaggedVariant,
    tag: &str,
    case_indent: &str,
    body_indent: &str,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = variant.csharp_name.to_lowercase();

    match &variant.data {
        TaggedVariantData::Unit => {
            let arm = format!(
                "{case_indent}case {csharp_name}:\n\
                 {body_indent}writer.WriteString(\"{tag}\", \"{json_name}\");\n\
                 {body_indent}break;",
            );
            quote! { String::from(#arm) }
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
                        ci = #case_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = #body_indent,
                        tag = #tag,
                        json = #json_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_stj_write_struct_arm(
            csharp_name,
            json_name,
            &var_name,
            fields,
            tag,
            case_indent,
            body_indent,
        ),
    }
}

/// Builds a STJ Write switch arm for a struct variant with named fields.
fn build_stj_write_struct_arm(
    csharp_name: &str,
    json_name: &str,
    var_name: &str,
    fields: &[CSharpField],
    tag: &str,
    case_indent: &str,
    body_indent: &str,
) -> TokenStream {
    let field_lines: Vec<String> = fields
        .iter()
        .map(|f| {
            let json = &f.json_name;
            let prop = &f.csharp_property_name;
            format!(
                "{body_indent}writer.WritePropertyName(\"{json}\");\n\
                 {body_indent}JsonSerializer.Serialize(writer, {var_name}.{prop}, options);",
            )
        })
        .collect();

    let fields_block = field_lines.join("\n");

    let arm = format!(
        "{case_indent}case {csharp_name} {var_name}:\n\
         {body_indent}writer.WriteString(\"{tag}\", \"{json_name}\");\n\
         {fields_block}\n\
         {body_indent}break;",
    );

    quote! { String::from(#arm) }
}

/// Builds a single Newtonsoft Read switch arm for a variant.
fn build_newtonsoft_read_arm(
    variant: &TaggedVariant,
    arm_indent: &str,
    prop_indent: &str,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            let arm = format!("{arm_indent}\"{json_name}\" => new {csharp_name}(),");
            quote! { String::from(#arm) }
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
                        arm = #arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        prop = #prop_indent,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_newtonsoft_read_struct_arm(
            csharp_name,
            json_name,
            fields,
            arm_indent,
            prop_indent,
        ),
    }
}

/// Builds a Newtonsoft Read switch arm for a struct variant with named fields.
fn build_newtonsoft_read_struct_arm(
    csharp_name: &str,
    json_name: &str,
    fields: &[CSharpField],
    arm_indent: &str,
    prop_indent: &str,
) -> TokenStream {
    let field_exprs: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let prop_name = &f.csharp_property_name;
            let field_json = &f.json_name;
            let type_expr = &f.type_expr;

            quote! {
                {
                    let csharp_type = #type_expr;
                    format!(
                        "{prop}{name} = obj[\"{json}\"].ToObject<{ty}>(serializer),",
                        prop = #prop_indent,
                        name = #prop_name,
                        json = #field_json,
                        ty = csharp_type,
                    )
                }
            }
        })
        .collect();

    quote! {
        {
            let field_lines: Vec<String> = vec![#(#field_exprs),*];
            let fields_str = field_lines.join("\n");
            format!(
                "{arm}\"{json}\" => new {name}\n\
                 {arm}{{\n\
                 {fields}\n\
                 {arm}}},",
                arm = #arm_indent,
                json = #json_name,
                name = #csharp_name,
                fields = fields_str,
            )
        }
    }
}

/// Builds a single Newtonsoft Write switch arm for a variant.
fn build_newtonsoft_write_arm(
    variant: &TaggedVariant,
    tag: &str,
    case_indent: &str,
    body_indent: &str,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = variant.csharp_name.to_lowercase();

    match &variant.data {
        TaggedVariantData::Unit => {
            let arm = format!(
                "{case_indent}case {csharp_name}:\n\
                 {body_indent}writer.WritePropertyName(\"{tag}\");\n\
                 {body_indent}writer.WriteValue(\"{json_name}\");\n\
                 {body_indent}break;",
            );
            quote! { String::from(#arm) }
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
                        ci = #case_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = #body_indent,
                        tag = #tag,
                        json = #json_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_newtonsoft_write_struct_arm(
            csharp_name,
            json_name,
            &var_name,
            fields,
            tag,
            case_indent,
            body_indent,
        ),
    }
}

/// Builds a Newtonsoft Write switch arm for a struct variant with named fields.
fn build_newtonsoft_write_struct_arm(
    csharp_name: &str,
    json_name: &str,
    var_name: &str,
    fields: &[CSharpField],
    tag: &str,
    case_indent: &str,
    body_indent: &str,
) -> TokenStream {
    let field_lines: Vec<String> = fields
        .iter()
        .map(|f| {
            let json = &f.json_name;
            let prop = &f.csharp_property_name;
            format!(
                "{body_indent}writer.WritePropertyName(\"{json}\");\n\
                 {body_indent}serializer.Serialize(writer, {var_name}.{prop});",
            )
        })
        .collect();

    let fields_block = field_lines.join("\n");

    let arm = format!(
        "{case_indent}case {csharp_name} {var_name}:\n\
         {body_indent}writer.WritePropertyName(\"{tag}\");\n\
         {body_indent}writer.WriteValue(\"{json_name}\");\n\
         {fields_block}\n\
         {body_indent}break;",
    );

    quote! { String::from(#arm) }
}

/// Builds the final token stream for file-scoped namespace output (C# 10+).
fn build_file_scoped(
    using_block: &str,
    namespace: &str,
    class_attrs: &str,
    csharp_name: &str,
    indent: &str,
    variant_exprs: &[TokenStream],
    converter_expr: Option<&TokenStream>,
) -> TokenStream {
    let converter_append = build_converter_append(converter_expr);

    quote! {
        {
            let variant_parts: Vec<String> = vec![#(#variant_exprs),*];
            let variants_block = variant_parts.join("\n\n");
            let converter_block: String = #converter_append;

            let body = if variants_block.is_empty() && converter_block.is_empty() {
                String::from(";")
            } else {
                format!(
                    "\n{{\n{variants}{converter}\n}}",
                    variants = variants_block,
                    converter = converter_block,
                )
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
    converter_expr: Option<&TokenStream>,
) -> TokenStream {
    let converter_append = build_converter_append(converter_expr);

    quote! {
        {
            let variant_parts: Vec<String> = vec![#(#variant_exprs),*];
            let variants_block = variant_parts.join("\n\n");
            let converter_block: String = #converter_append;

            let body = if variants_block.is_empty() && converter_block.is_empty() {
                String::from(";")
            } else {
                format!(
                    "\n    {{\n{variants}{converter}\n    }}",
                    variants = variants_block,
                    converter = converter_block,
                )
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

/// Builds a token stream expression for the converter append string.
///
/// Returns a `quote!` block evaluating to an empty `String` when no converter
/// is needed, or to the converter class body otherwise.
fn build_converter_append(converter_expr: Option<&TokenStream>) -> TokenStream {
    if let Some(expr) = converter_expr {
        quote! { #expr }
    } else {
        quote! { String::new() }
    }
}
