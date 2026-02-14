// Rust guideline compliant 2026-02-14
//! Tagged enum code generation (internally, adjacently, externally tagged, and
//! untagged).
//!
//! Generates C# abstract record type hierarchies with sealed derived records.
//! Dispatches to the appropriate code generation strategy based on the
//! [`EnumTagging`] variant and the target C# version / serializer combination.
//!
//! When native polymorphism is not available (STJ C# 9-10, or Newtonsoft at
//! any version), a custom `JsonConverter<T>` is generated as a nested
//! `private sealed class` inside the abstract record.

use crate::config::{CSharpConfig, CSharpVersion, Serializer};
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

    // A HashMap flatten field always emits a nullable `Dictionary<…>?
    // ExtensionData` reference-type property, so the file needs `#nullable
    // enable` unconditionally when any variant contains one.
    // Also determines whether the native polymorphism using block needs
    // `System.Text.Json` for `JsonElement`.
    let has_hashmap_flatten = variants.iter().any(|v| match &v.data {
        TaggedVariantData::Struct(fields) => fields
            .iter()
            .any(|f| matches!(f.flatten, FlattenKind::HashMap { .. })),
        _ => false,
    });

    let using_block = build_using_block(config, use_native_polymorphism, has_hashmap_flatten);
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

    // Collect nullable-reference-type checks from all struct variant fields.
    let nullable_checks: Vec<TokenStream> = variants
        .iter()
        .flat_map(|v| match &v.data {
            TaggedVariantData::Struct(fields) => super::nullable_ref_check_exprs(fields),
            _ => Vec::new(),
        })
        .collect();

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
            has_hashmap_flatten,
            &nullable_checks,
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
            has_hashmap_flatten,
            &nullable_checks,
        )
    }
}

/// Determines the `using` directives for the generated file.
fn build_using_block(
    config: &CSharpConfig,
    use_native_polymorphism: bool,
    has_hashmap_flatten: bool,
) -> String {
    if use_native_polymorphism {
        if has_hashmap_flatten {
            // Need System.Text.Json for JsonElement used by [JsonExtensionData].
            return String::from("using System.Text.Json;\nusing System.Text.Json.Serialization;");
        }
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
    let prop_indent = format!("{base_indent}    ");
    let field_exprs = build_struct_variant_fields(fields, config, &prop_indent, use_required);

    quote! {
        {
            let mut field_parts: Vec<String> = Vec::new();
            #(#field_exprs)*
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

/// Builds the property declaration token streams for a struct variant's fields.
///
/// Dispatches on [`FlattenKind`] for each field:
/// - `None`: standard `[JsonPropertyName]` + property declaration
/// - `Struct`: runtime iteration over `csharp_fields()` with property + extension data handling
/// - `HashMap`: `[JsonExtensionData]` dictionary property
fn build_struct_variant_fields(
    fields: &[CSharpField],
    config: &CSharpConfig,
    prop_indent: &str,
    use_required: bool,
) -> Vec<TokenStream> {
    let attr_name = json_attr_name(config);
    let required_kw = if use_required { "required " } else { "" };

    let extension_data_type = match config.serializer {
        Serializer::SystemTextJson => "Dictionary<string, JsonElement>",
        Serializer::Newtonsoft => "Dictionary<string, JToken>",
    };

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
                                let req = if is_optional { "" } else { #required_kw };
                                field_parts.push(format!(
                                    "{prop}[{attr}(\"{json}\")]\n\
                                     {prop}public {req}{ty}{null} {name} {{ get; init; }}",
                                    prop = #prop_indent,
                                    attr = #attr_name,
                                    json = json_name,
                                    req = req,
                                    ty = type_name,
                                    null = nullable,
                                    name = property_name,
                                ));
                            }
                            csharp_rs::CSharpFieldInfo::ExtensionData { .. } => {
                                field_parts.push(format!(
                                    "{prop}[JsonExtensionData]\n\
                                     {prop}public {ext_type}? ExtensionData {{ get; set; }}",
                                    prop = #prop_indent,
                                    ext_type = #extension_data_type,
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
                        prop = #prop_indent,
                        ext_type = #extension_data_type,
                    ));
                }
            }
        })
        .collect()
}

/// Returns the JSON attribute name for a property based on the serializer.
fn json_attr_name(config: &CSharpConfig) -> &'static str {
    match config.serializer {
        Serializer::SystemTextJson => "JsonPropertyName",
        Serializer::Newtonsoft => "JsonProperty",
    }
}

/// Builds per-field write expressions for converter struct arms.
///
/// Returns `Vec<TokenStream>` where each item is a statement pushing
/// onto a pre-existing `field_lines: Vec<String>`. Handles all three
/// [`FlattenKind`] variants:
/// - `None`: compile-time property write
/// - `Struct`: runtime iteration over `csharp_fields()`
/// - `HashMap`: skipped (extension data handled by serializer)
fn build_write_field_exprs(
    fields: &[CSharpField],
    var_name: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> Vec<TokenStream> {
    let serialize_call = match config.serializer {
        Serializer::SystemTextJson => "JsonSerializer.Serialize(writer, {var}.{prop}, options);",
        Serializer::Newtonsoft => "serializer.Serialize(writer, {var}.{prop});",
    };

    fields
        .iter()
        .filter_map(|f| {
            match &f.flatten {
                FlattenKind::None => {
                    let json = &f.json_name;
                    let prop = &f.csharp_property_name;
                    Some(quote! {
                        field_lines.push(format!(
                            concat!(
                                "{body}writer.WritePropertyName(\"{json}\");\n",
                                "{body}", #serialize_call,
                            ),
                            body = #body_indent,
                            json = #json,
                            var = #var_name,
                            prop = #prop,
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
                                        concat!(
                                            "{body}writer.WritePropertyName(\"{json}\");\n",
                                            "{body}", #serialize_call,
                                        ),
                                        body = #body_indent,
                                        json = json_name,
                                        var = #var_name,
                                        prop = property_name,
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
/// `{json}`, `{ty}` — e.g.:
/// - STJ internal: `"{indent}{name} = root.GetProperty(\"{json}\").Deserialize<{ty}>(options),"`
/// - Newtonsoft external: `"{indent}{name} = obj[\"{json}\"].ToObject<{ty}>(serializer),"`
///
/// Returns `Vec<TokenStream>` where each item pushes onto `field_lines`.
fn build_read_field_exprs(
    fields: &[CSharpField],
    indent: &str,
    read_fmt: &str,
) -> Vec<TokenStream> {
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
                                indent = #indent,
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
                                        indent = #indent,
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
                    config,
                )),
                Serializer::Newtonsoft => Some(build_internal_newtonsoft_converter(
                    csharp_name,
                    tag,
                    variants,
                    base_indent,
                    config,
                )),
            }
        }
        EnumTagging::External => {
            let base_indent = if use_file_scoped_ns {
                "    "
            } else {
                "        "
            };

            match config.serializer {
                Serializer::SystemTextJson => Some(build_external_stj_converter(
                    csharp_name,
                    variants,
                    base_indent,
                    config,
                )),
                Serializer::Newtonsoft => Some(build_external_newtonsoft_converter(
                    csharp_name,
                    variants,
                    base_indent,
                    config,
                )),
            }
        }
        EnumTagging::Adjacent { tag, content } => {
            let base_indent = if use_file_scoped_ns {
                "    "
            } else {
                "        "
            };

            match config.serializer {
                Serializer::SystemTextJson => Some(build_adjacent_stj_converter(
                    csharp_name,
                    tag,
                    content,
                    variants,
                    base_indent,
                    config,
                )),
                Serializer::Newtonsoft => Some(build_adjacent_newtonsoft_converter(
                    csharp_name,
                    tag,
                    content,
                    variants,
                    base_indent,
                    config,
                )),
            }
        }
        EnumTagging::Untagged => {
            let base_indent = if use_file_scoped_ns {
                "    "
            } else {
                "        "
            };

            match config.serializer {
                Serializer::SystemTextJson => Some(build_untagged_stj_converter(
                    csharp_name,
                    variants,
                    base_indent,
                    config,
                )),
                Serializer::Newtonsoft => Some(build_untagged_newtonsoft_converter(
                    csharp_name,
                    variants,
                    base_indent,
                    config,
                )),
            }
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
    config: &CSharpConfig,
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
        .map(|v| build_stj_write_arm(v, tag, &inner3, &inner4, config))
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
    config: &CSharpConfig,
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
        .map(|v| build_newtonsoft_write_arm(v, tag, &inner3, &inner4, config))
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
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular property deserialized from `root.GetProperty(...)`.
/// - `Struct`: iterates `csharp_fields()` at runtime and inlines each property.
/// - `HashMap`: skipped (extension data is not read back).
fn build_stj_read_struct_arm(
    csharp_name: &str,
    json_name: &str,
    fields: &[CSharpField],
    arm_indent: &str,
    prop_indent: &str,
) -> TokenStream {
    let read_fmt = "{indent}{name} = root.GetProperty(\"{json}\").Deserialize<{ty}>(options),";
    let field_exprs = build_read_field_exprs(fields, prop_indent, read_fmt);

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
    config: &CSharpConfig,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

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
            config,
        ),
    }
}

/// Builds a STJ Write switch arm for a struct variant with named fields.
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular compile-time property write
/// - `Struct`: runtime iteration over `csharp_fields()` properties
/// - `HashMap`: skipped (extension data handled by serializer)
#[expect(
    clippy::too_many_arguments,
    reason = "internal tagging needs tag key in addition to standard arm params plus config"
)]
fn build_stj_write_struct_arm(
    csharp_name: &str,
    json_name: &str,
    var_name: &str,
    fields: &[CSharpField],
    tag: &str,
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let field_exprs = build_write_field_exprs(fields, var_name, body_indent, config);

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
                case = #case_indent,
                name = #csharp_name,
                var = #var_name,
                body = #body_indent,
                tag = #tag,
                json = #json_name,
                fields = fields_block,
            )
        }
    }
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
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular property deserialized from `obj["{json}"]`.
/// - `Struct`: iterates `csharp_fields()` at runtime and inlines each property.
/// - `HashMap`: skipped (extension data is not read back).
fn build_newtonsoft_read_struct_arm(
    csharp_name: &str,
    json_name: &str,
    fields: &[CSharpField],
    arm_indent: &str,
    prop_indent: &str,
) -> TokenStream {
    let read_fmt = "{indent}{name} = obj[\"{json}\"].ToObject<{ty}>(serializer),";
    let field_exprs = build_read_field_exprs(fields, prop_indent, read_fmt);

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
    config: &CSharpConfig,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

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
            config,
        ),
    }
}

/// Builds a Newtonsoft Write switch arm for a struct variant with named fields.
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular compile-time property write
/// - `Struct`: runtime iteration over `csharp_fields()` properties
/// - `HashMap`: skipped (extension data handled by serializer)
#[expect(
    clippy::too_many_arguments,
    reason = "internal tagging needs tag key in addition to standard arm params plus config"
)]
fn build_newtonsoft_write_struct_arm(
    csharp_name: &str,
    json_name: &str,
    var_name: &str,
    fields: &[CSharpField],
    tag: &str,
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let field_exprs = build_write_field_exprs(fields, var_name, body_indent, config);

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
                case = #case_indent,
                name = #csharp_name,
                var = #var_name,
                body = #body_indent,
                tag = #tag,
                json = #json_name,
                fields = fields_block,
            )
        }
    }
}

/// Builds the STJ `JsonConverter<T>` for externally tagged enums.
///
/// External tagging is serde's default: unit variants serialize to raw strings
/// (e.g., `"Quit"`), while data variants serialize to single-property objects
/// (e.g., `{"Text": "hello"}` or `{"Request": {"id": "abc"}}`).
///
/// The `Read` method checks `root.ValueKind` to distinguish string (unit) from
/// object (data) variants. The `Write` method uses `WriteStringValue` for unit
/// variants and wraps data variants in an object keyed by the variant name.
fn build_external_stj_converter(
    csharp_name: &str,
    variants: &[TaggedVariant],
    base_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let inner = format!("{base_indent}    ");
    let inner2 = format!("{inner}    ");
    let inner3 = format!("{inner2}    ");
    let inner4 = format!("{inner3}    ");
    let inner5 = format!("{inner4}    ");

    let read_unit_arms: Vec<TokenStream> = variants
        .iter()
        .filter(|v| matches!(v.data, TaggedVariantData::Unit))
        .map(|v| build_external_stj_read_unit_arm(v, &inner4, &inner5))
        .collect();

    let read_object_arms: Vec<TokenStream> = variants
        .iter()
        .filter(|v| !matches!(v.data, TaggedVariantData::Unit))
        .map(|v| build_external_stj_read_object_arm(v, &inner4, &inner5))
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_external_stj_write_arm(v, &inner3, &inner4, config))
        .collect();

    quote! {
        {
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
                 {base}private sealed class {name}Converter : JsonConverter<{name}>\n\
                 {base}{{\n\
                 {i1}public override {name} Read(\n\
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
                 {i2}{name} value,\n\
                 {i2}JsonSerializerOptions options)\n\
                 {i1}{{\n\
                 {i2}switch (value)\n\
                 {i2}{{\n\
                 {write}\n\
                 {i2}}}\n\
                 {i1}}}\n\
                 {base}}}",
                base = #base_indent,
                name = #csharp_name,
                i1 = #inner,
                i2 = #inner2,
                i3 = #inner3,
                read_unit = read_unit_block,
                read_obj = read_object_block,
                write = write_block,
            )
        }
    }
}

/// Builds a STJ Read switch arm for a unit variant in external tagging.
///
/// Unit variants appear as raw JSON strings (e.g., `"Quit"`).
fn build_external_stj_read_unit_arm(
    variant: &TaggedVariant,
    arm_indent: &str,
    _prop_indent: &str,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    let arm = format!("{arm_indent}\"{json_name}\" => new {csharp_name}(),");
    quote! { String::from(#arm) }
}

/// Builds a STJ Read switch arm for a data variant (newtype or struct) in
/// external tagging.
///
/// Data variants appear as single-property objects: `{"VariantName": data}`.
fn build_external_stj_read_object_arm(
    variant: &TaggedVariant,
    arm_indent: &str,
    prop_indent: &str,
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
                         {prop}Value = prop.Value.Deserialize<{ty}>(options),\n\
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
            let read_fmt =
                "{indent}{name} = prop.Value.GetProperty(\"{json}\").Deserialize<{ty}>(options),";
            let field_exprs = build_read_field_exprs(fields, prop_indent, read_fmt);

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
                        arm = #arm_indent,
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
fn build_external_stj_write_arm(
    variant: &TaggedVariant,
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            let arm = format!(
                "{case_indent}case {csharp_name}:\n\
                 {body_indent}writer.WriteStringValue(\"{json_name}\");\n\
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
                         {bi}writer.WriteStartObject();\n\
                         {bi}writer.WritePropertyName(\"{json}\");\n\
                         {bi}JsonSerializer.Serialize(writer, {var}.Value, options);\n\
                         {bi}writer.WriteEndObject();\n\
                         {bi}break;",
                        ci = #case_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = #body_indent,
                        json = #json_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_external_stj_write_struct_arm(
            csharp_name,
            json_name,
            &var_name,
            fields,
            case_indent,
            body_indent,
            config,
        ),
    }
}

/// Builds a STJ Write switch arm for a struct variant in external tagging.
///
/// Wraps the struct fields in a nested object keyed by the variant name:
/// `{"VariantName": {"field1": ..., "field2": ...}}`.
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular compile-time property write
/// - `Struct`: runtime iteration over `csharp_fields()` properties
/// - `HashMap`: skipped (extension data handled by serializer)
fn build_external_stj_write_struct_arm(
    csharp_name: &str,
    json_name: &str,
    var_name: &str,
    fields: &[CSharpField],
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let field_exprs = build_write_field_exprs(fields, var_name, body_indent, config);

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
                case = #case_indent,
                name = #csharp_name,
                var = #var_name,
                body = #body_indent,
                json = #json_name,
                fields = fields_block,
            )
        }
    }
}

/// Builds the Newtonsoft `JsonConverter<T>` for externally tagged enums.
///
/// External tagging uses `JsonToken.String` for unit variants and
/// `JsonToken.StartObject` for data variants. The Read method dispatches
/// on `reader.TokenType`, while Write uses `WriteValue` for units and
/// `WritePropertyName` + `serializer.Serialize` for data variants.
fn build_external_newtonsoft_converter(
    csharp_name: &str,
    variants: &[TaggedVariant],
    base_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let inner = format!("{base_indent}    ");
    let inner2 = format!("{inner}    ");
    let inner3 = format!("{inner2}    ");
    let inner4 = format!("{inner3}    ");
    let inner5 = format!("{inner4}    ");

    let read_unit_arms: Vec<TokenStream> = variants
        .iter()
        .filter(|v| matches!(v.data, TaggedVariantData::Unit))
        .map(|v| build_external_newtonsoft_read_unit_arm(v, &inner4, &inner5))
        .collect();

    let read_object_arms: Vec<TokenStream> = variants
        .iter()
        .filter(|v| !matches!(v.data, TaggedVariantData::Unit))
        .map(|v| build_external_newtonsoft_read_object_arm(v, &inner4, &inner5))
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_external_newtonsoft_write_arm(v, &inner3, &inner4, config))
        .collect();

    quote! {
        {
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
                base = #base_indent,
                name = #csharp_name,
                i1 = #inner,
                i2 = #inner2,
                i3 = #inner3,
                read_unit = read_unit_block,
                read_obj = read_object_block,
                write = write_block,
            )
        }
    }
}

/// Builds a Newtonsoft Read switch arm for a unit variant in external tagging.
fn build_external_newtonsoft_read_unit_arm(
    variant: &TaggedVariant,
    arm_indent: &str,
    _prop_indent: &str,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;

    let arm = format!("{arm_indent}\"{json_name}\" => new {csharp_name}(),");
    quote! { String::from(#arm) }
}

/// Builds a Newtonsoft Read switch arm for a data variant (newtype or struct)
/// in external tagging.
fn build_external_newtonsoft_read_object_arm(
    variant: &TaggedVariant,
    arm_indent: &str,
    prop_indent: &str,
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
            let read_fmt = "{indent}{name} = prop.Value[\"{json}\"].ToObject<{ty}>(serializer),";
            let field_exprs = build_read_field_exprs(fields, prop_indent, read_fmt);

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
                        arm = #arm_indent,
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
fn build_external_newtonsoft_write_arm(
    variant: &TaggedVariant,
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            let arm = format!(
                "{case_indent}case {csharp_name}:\n\
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
                         {bi}writer.WriteStartObject();\n\
                         {bi}writer.WritePropertyName(\"{json}\");\n\
                         {bi}serializer.Serialize(writer, {var}.Value);\n\
                         {bi}writer.WriteEndObject();\n\
                         {bi}break;",
                        ci = #case_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = #body_indent,
                        json = #json_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_external_newtonsoft_write_struct_arm(
            csharp_name,
            json_name,
            &var_name,
            fields,
            case_indent,
            body_indent,
            config,
        ),
    }
}

/// Builds a Newtonsoft Write switch arm for a struct variant in external
/// tagging.
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular compile-time property write
/// - `Struct`: runtime iteration over `csharp_fields()` properties
/// - `HashMap`: skipped (extension data handled by serializer)
fn build_external_newtonsoft_write_struct_arm(
    csharp_name: &str,
    json_name: &str,
    var_name: &str,
    fields: &[CSharpField],
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let field_exprs = build_write_field_exprs(fields, var_name, body_indent, config);

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
                case = #case_indent,
                name = #csharp_name,
                var = #var_name,
                body = #body_indent,
                json = #json_name,
                fields = fields_block,
            )
        }
    }
}

/// Builds the STJ `JsonConverter<T>` for adjacently tagged enums.
///
/// Adjacent tagging uses `#[serde(tag = "...", content = "...")]` where the
/// discriminator and payload are sibling properties in a flat object. Unit
/// variants omit the content key entirely. Newtype variants place the value
/// directly under the content key. Struct variants nest their fields inside
/// the content object.
fn build_adjacent_stj_converter(
    csharp_name: &str,
    tag: &str,
    content: &str,
    variants: &[TaggedVariant],
    base_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let inner = format!("{base_indent}    ");
    let inner2 = format!("{inner}    ");
    let inner3 = format!("{inner2}    ");
    let inner4 = format!("{inner3}    ");

    let read_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_adjacent_stj_read_arm(v, content, &inner3, &inner4))
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_adjacent_stj_write_arm(v, tag, content, &inner3, &inner4, config))
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
                 {i2}switch (value)\n\
                 {i2}{{\n\
                 {write}\n\
                 {i2}}}\n\
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

/// Builds a single STJ Read switch arm for a variant in adjacent tagging.
///
/// Unit variants only check the tag (no content property). Newtype variants
/// deserialize the content property directly. Struct variants extract fields
/// from the content object.
fn build_adjacent_stj_read_arm(
    variant: &TaggedVariant,
    content: &str,
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
                         {prop}Value = root.GetProperty(\"{content}\").Deserialize<{ty}>(options),\n\
                         {arm}}},",
                        arm = #arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        prop = #prop_indent,
                        content = #content,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_adjacent_stj_read_struct_arm(
            csharp_name,
            json_name,
            fields,
            content,
            arm_indent,
            prop_indent,
        ),
    }
}

/// Builds a STJ Read switch arm for a struct variant in adjacent tagging.
///
/// Reads `root.GetProperty("{content}")` into a content element, then
/// extracts each field from that content object.
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular property deserialized from the content sub-element.
/// - `Struct`: iterates `csharp_fields()` at runtime and inlines each property.
/// - `HashMap`: skipped (extension data is not read back).
fn build_adjacent_stj_read_struct_arm(
    csharp_name: &str,
    json_name: &str,
    fields: &[CSharpField],
    content: &str,
    arm_indent: &str,
    prop_indent: &str,
) -> TokenStream {
    let read_fmt = format!(
        "{{indent}}{{name}} = root.GetProperty(\"{content}\").GetProperty(\"{{json}}\").Deserialize<{{ty}}>(options),"
    );
    let field_exprs = build_read_field_exprs(fields, prop_indent, &read_fmt);

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
                arm = #arm_indent,
                json = #json_name,
                name = #csharp_name,
                fields = fields_str,
            )
        }
    }
}

/// Builds a single STJ Write switch arm for a variant in adjacent tagging.
///
/// All variants write the tag property. Unit variants omit the content key.
/// Newtype variants write the content as a direct value. Struct variants
/// nest their fields inside a content object.
fn build_adjacent_stj_write_arm(
    variant: &TaggedVariant,
    tag: &str,
    content: &str,
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            // Unit: write start object, tag string, end object (no content key).
            let arm = format!(
                "{case_indent}case {csharp_name}:\n\
                 {body_indent}writer.WriteStartObject();\n\
                 {body_indent}writer.WriteString(\"{tag}\", \"{json_name}\");\n\
                 {body_indent}writer.WriteEndObject();\n\
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
                         {bi}writer.WriteStartObject();\n\
                         {bi}writer.WriteString(\"{tag}\", \"{json}\");\n\
                         {bi}writer.WritePropertyName(\"{content}\");\n\
                         {bi}JsonSerializer.Serialize(writer, {var}.Value, options);\n\
                         {bi}writer.WriteEndObject();\n\
                         {bi}break;",
                        ci = #case_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = #body_indent,
                        tag = #tag,
                        json = #json_name,
                        content = #content,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_adjacent_stj_write_struct_arm(
            csharp_name,
            json_name,
            &var_name,
            fields,
            tag,
            content,
            case_indent,
            body_indent,
            config,
        ),
    }
}

/// Builds a STJ Write switch arm for a struct variant in adjacent tagging.
///
/// Writes an outer object with the tag, then a content property containing
/// a nested object with the struct fields.
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular compile-time property write
/// - `Struct`: runtime iteration over `csharp_fields()` properties
/// - `HashMap`: skipped (extension data handled by serializer)
#[expect(
    clippy::too_many_arguments,
    reason = "adjacent tagging needs tag + content keys in addition to standard arm params"
)]
fn build_adjacent_stj_write_struct_arm(
    csharp_name: &str,
    json_name: &str,
    var_name: &str,
    fields: &[CSharpField],
    tag: &str,
    content: &str,
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let field_exprs = build_write_field_exprs(fields, var_name, body_indent, config);

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
                case = #case_indent,
                name = #csharp_name,
                var = #var_name,
                body = #body_indent,
                tag = #tag,
                json = #json_name,
                content = #content,
                fields = fields_block,
            )
        }
    }
}

/// Builds the Newtonsoft `JsonConverter<T>` for adjacently tagged enums.
///
/// Uses `JObject.Load(reader)` for deserialization and `JsonWriter` for
/// serialization. The tag and content keys are sibling properties in the
/// JSON object.
fn build_adjacent_newtonsoft_converter(
    csharp_name: &str,
    tag: &str,
    content: &str,
    variants: &[TaggedVariant],
    base_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let inner = format!("{base_indent}    ");
    let inner2 = format!("{inner}    ");
    let inner3 = format!("{inner2}    ");
    let inner4 = format!("{inner3}    ");

    let read_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_adjacent_newtonsoft_read_arm(v, content, &inner3, &inner4))
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_adjacent_newtonsoft_write_arm(v, tag, content, &inner3, &inner4, config))
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
                 {i2}switch (value)\n\
                 {i2}{{\n\
                 {write}\n\
                 {i2}}}\n\
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

/// Builds a single Newtonsoft Read switch arm for a variant in adjacent
/// tagging.
///
/// Unit variants construct directly from the tag match. Newtype variants
/// use `obj["{content}"].ToObject<T>`. Struct variants extract fields from
/// the content sub-object.
fn build_adjacent_newtonsoft_read_arm(
    variant: &TaggedVariant,
    content: &str,
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
                         {prop}Value = obj[\"{content}\"].ToObject<{ty}>(serializer),\n\
                         {arm}}},",
                        arm = #arm_indent,
                        json = #json_name,
                        name = #csharp_name,
                        prop = #prop_indent,
                        content = #content,
                        ty = csharp_type,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_adjacent_newtonsoft_read_struct_arm(
            csharp_name,
            json_name,
            fields,
            content,
            arm_indent,
            prop_indent,
        ),
    }
}

/// Builds a Newtonsoft Read switch arm for a struct variant in adjacent
/// tagging.
///
/// Reads the content sub-object via `obj["{content}"]` and extracts each
/// field from it.
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular property deserialized from the content sub-object.
/// - `Struct`: iterates `csharp_fields()` at runtime and inlines each property.
/// - `HashMap`: skipped (extension data is not read back).
fn build_adjacent_newtonsoft_read_struct_arm(
    csharp_name: &str,
    json_name: &str,
    fields: &[CSharpField],
    content: &str,
    arm_indent: &str,
    prop_indent: &str,
) -> TokenStream {
    let read_fmt = format!(
        "{{indent}}{{name}} = obj[\"{content}\"][\"{{json}}\"].ToObject<{{ty}}>(serializer),"
    );
    let field_exprs = build_read_field_exprs(fields, prop_indent, &read_fmt);

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
                arm = #arm_indent,
                json = #json_name,
                name = #csharp_name,
                fields = fields_str,
            )
        }
    }
}

/// Builds a single Newtonsoft Write switch arm for a variant in adjacent
/// tagging.
///
/// Unit variants write only the tag property. Newtype and struct variants
/// write both the tag and content properties, with struct content wrapped
/// in a nested object.
fn build_adjacent_newtonsoft_write_arm(
    variant: &TaggedVariant,
    tag: &str,
    content: &str,
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let json_name = &variant.json_name;
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            // Unit: write start object, tag property + value, end object.
            let arm = format!(
                "{case_indent}case {csharp_name}:\n\
                 {body_indent}writer.WriteStartObject();\n\
                 {body_indent}writer.WritePropertyName(\"{tag}\");\n\
                 {body_indent}writer.WriteValue(\"{json_name}\");\n\
                 {body_indent}writer.WriteEndObject();\n\
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
                         {bi}writer.WriteStartObject();\n\
                         {bi}writer.WritePropertyName(\"{tag}\");\n\
                         {bi}writer.WriteValue(\"{json}\");\n\
                         {bi}writer.WritePropertyName(\"{content}\");\n\
                         {bi}serializer.Serialize(writer, {var}.Value);\n\
                         {bi}writer.WriteEndObject();\n\
                         {bi}break;",
                        ci = #case_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = #body_indent,
                        tag = #tag,
                        json = #json_name,
                        content = #content,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_adjacent_newtonsoft_write_struct_arm(
            csharp_name,
            json_name,
            &var_name,
            fields,
            tag,
            content,
            case_indent,
            body_indent,
            config,
        ),
    }
}

/// Builds a Newtonsoft Write switch arm for a struct variant in adjacent
/// tagging.
///
/// Writes the tag, then a content property containing a nested object with
/// the struct fields.
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular compile-time property write
/// - `Struct`: runtime iteration over `csharp_fields()` properties
/// - `HashMap`: skipped (extension data handled by serializer)
#[expect(
    clippy::too_many_arguments,
    reason = "adjacent tagging needs tag + content keys in addition to standard arm params"
)]
fn build_adjacent_newtonsoft_write_struct_arm(
    csharp_name: &str,
    json_name: &str,
    var_name: &str,
    fields: &[CSharpField],
    tag: &str,
    content: &str,
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let field_exprs = build_write_field_exprs(fields, var_name, body_indent, config);

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
                case = #case_indent,
                name = #csharp_name,
                var = #var_name,
                body = #body_indent,
                tag = #tag,
                json = #json_name,
                content = #content,
                fields = fields_block,
            )
        }
    }
}

/// Builds the STJ `JsonConverter<T>` for untagged enums.
///
/// Untagged enums have no discriminator; deserialization tries each variant in
/// declaration order using try/catch. Unit variants serialize as `null`, newtype
/// variants serialize their value directly, and struct variants serialize as
/// flat objects without any wrapping.
fn build_untagged_stj_converter(
    csharp_name: &str,
    variants: &[TaggedVariant],
    base_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let inner = format!("{base_indent}    ");
    let inner2 = format!("{inner}    ");
    let inner3 = format!("{inner2}    ");

    let read_attempts: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_untagged_stj_read_attempt(v, &inner2, &inner3))
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_untagged_stj_write_arm(v, &inner3, &format!("{inner3}    "), config))
        .collect();

    quote! {
        {
            let mut read_parts: Vec<String> = Vec::new();
            #(read_parts.push(#read_attempts);)*

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
                 \n\
                 {read}\n\
                 \n\
                 {i2}throw new JsonException(\"No matching variant found for {name}\");\n\
                 {i1}}}\n\
                 \n\
                 {i1}public override void Write(\n\
                 {i2}Utf8JsonWriter writer,\n\
                 {i2}{name} value,\n\
                 {i2}JsonSerializerOptions options)\n\
                 {i1}{{\n\
                 {i2}switch (value)\n\
                 {i2}{{\n\
                 {write}\n\
                 {i2}}}\n\
                 {i1}}}\n\
                 {base}}}",
                base = #base_indent,
                name = #csharp_name,
                i1 = #inner,
                i2 = #inner2,
                read = read_block,
                write = write_block,
            )
        }
    }
}

/// Builds a single STJ Read attempt block for a variant in untagged mode.
///
/// Unit variants check `ValueKind.Null`. Data variants use a `try`/`catch`
/// block to attempt construction from the parsed JSON element. Variants are
/// tried in declaration order; the first successful match wins.
fn build_untagged_stj_read_attempt(
    variant: &TaggedVariant,
    attempt_indent: &str,
    body_indent: &str,
) -> TokenStream {
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            // Unit variants: check for null.
            let block = format!(
                "{attempt_indent}if (root.ValueKind == JsonValueKind.Null)\n\
                 {attempt_indent}{{\n\
                 {body_indent}return new {csharp_name}();\n\
                 {attempt_indent}}}",
            );
            quote! { String::from(#block) }
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
                        ai = #attempt_indent,
                        bi = #body_indent,
                        ty = csharp_type,
                        name = #csharp_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => {
            build_untagged_stj_read_struct_attempt(csharp_name, fields, attempt_indent, body_indent)
        }
    }
}

/// Builds a STJ Read try/catch block for a struct variant in untagged mode.
///
/// Attempts to extract each field via `root.GetProperty(...)`, constructing
/// the variant record if all fields are found.
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular property deserialized from the root element.
/// - `Struct`: iterates `csharp_fields()` at runtime and inlines each property.
/// - `HashMap`: skipped (extension data is not read back).
fn build_untagged_stj_read_struct_attempt(
    csharp_name: &str,
    fields: &[CSharpField],
    attempt_indent: &str,
    body_indent: &str,
) -> TokenStream {
    let read_fmt = "{indent}    {name} = root.GetProperty(\"{json}\").Deserialize<{ty}>(options),";
    let field_exprs = build_read_field_exprs(fields, body_indent, read_fmt);

    quote! {
        {
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
                ai = #attempt_indent,
                bi = #body_indent,
                name = #csharp_name,
                fields = fields_str,
            )
        }
    }
}

/// Builds a single STJ Write switch arm for a variant in untagged mode.
///
/// No discriminator is written. Unit variants emit `WriteNullValue()`. Newtype
/// variants serialize their value directly. Struct variants write a flat object
/// with their fields.
fn build_untagged_stj_write_arm(
    variant: &TaggedVariant,
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            let arm = format!(
                "{case_indent}case {csharp_name}:\n\
                 {body_indent}writer.WriteNullValue();\n\
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
                         {bi}JsonSerializer.Serialize(writer, {var}.Value, options);\n\
                         {bi}break;",
                        ci = #case_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = #body_indent,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_untagged_stj_write_struct_arm(
            csharp_name,
            &var_name,
            fields,
            case_indent,
            body_indent,
            config,
        ),
    }
}

/// Builds a STJ Write switch arm for a struct variant in untagged mode.
///
/// Writes a flat object containing just the struct fields, with no
/// discriminator or wrapping.
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular compile-time property write
/// - `Struct`: runtime iteration over `csharp_fields()` properties
/// - `HashMap`: skipped (extension data handled by serializer)
fn build_untagged_stj_write_struct_arm(
    csharp_name: &str,
    var_name: &str,
    fields: &[CSharpField],
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let field_exprs = build_write_field_exprs(fields, var_name, body_indent, config);

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
                case = #case_indent,
                name = #csharp_name,
                var = #var_name,
                body = #body_indent,
                fields = fields_block,
            )
        }
    }
}

/// Builds the Newtonsoft `JsonConverter<T>` for untagged enums.
///
/// Uses `JToken.ReadFrom(reader)` for buffered deserialization and tries each
/// variant in declaration order. Unit variants check `JTokenType.Null`. Data
/// variants use try/catch. Write emits raw content with no discriminator.
fn build_untagged_newtonsoft_converter(
    csharp_name: &str,
    variants: &[TaggedVariant],
    base_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let inner = format!("{base_indent}    ");
    let inner2 = format!("{inner}    ");
    let inner3 = format!("{inner2}    ");

    let read_attempts: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_untagged_newtonsoft_read_attempt(v, &inner2, &inner3))
        .collect();

    let write_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| build_untagged_newtonsoft_write_arm(v, &inner3, &format!("{inner3}    "), config))
        .collect();

    quote! {
        {
            let mut read_parts: Vec<String> = Vec::new();
            #(read_parts.push(#read_attempts);)*

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
                base = #base_indent,
                name = #csharp_name,
                i1 = #inner,
                i2 = #inner2,
                read = read_block,
                write = write_block,
            )
        }
    }
}

/// Builds a single Newtonsoft Read attempt block for a variant in untagged
/// mode.
///
/// Unit variants check `token.Type == JTokenType.Null`. Newtype variants use
/// `token.ToObject<T>(serializer)`. Struct variants cast to `JObject` and
/// extract fields. All data variants are wrapped in try/catch.
fn build_untagged_newtonsoft_read_attempt(
    variant: &TaggedVariant,
    attempt_indent: &str,
    body_indent: &str,
) -> TokenStream {
    let csharp_name = &variant.csharp_name;

    match &variant.data {
        TaggedVariantData::Unit => {
            // Unit variants: check for null token.
            let block = format!(
                "{attempt_indent}if (token.Type == JTokenType.Null)\n\
                 {attempt_indent}{{\n\
                 {body_indent}return new {csharp_name}();\n\
                 {attempt_indent}}}",
            );
            quote! { String::from(#block) }
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
                        ai = #attempt_indent,
                        bi = #body_indent,
                        ty = csharp_type,
                        name = #csharp_name,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_untagged_newtonsoft_read_struct_attempt(
            csharp_name,
            fields,
            attempt_indent,
            body_indent,
        ),
    }
}

/// Builds a Newtonsoft Read try/catch block for a struct variant in untagged
/// mode.
///
/// Casts the token to `JObject` and extracts each field via indexing and
/// `ToObject<T>(serializer)`.
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular property deserialized from the `JObject`.
/// - `Struct`: iterates `csharp_fields()` at runtime and inlines each property.
/// - `HashMap`: skipped (extension data is not read back).
fn build_untagged_newtonsoft_read_struct_attempt(
    csharp_name: &str,
    fields: &[CSharpField],
    attempt_indent: &str,
    body_indent: &str,
) -> TokenStream {
    let read_fmt = "{indent}    {name} = obj[\"{json}\"].ToObject<{ty}>(serializer),";
    let field_exprs = build_read_field_exprs(fields, body_indent, read_fmt);

    quote! {
        {
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
                ai = #attempt_indent,
                bi = #body_indent,
                name = #csharp_name,
                fields = fields_str,
            )
        }
    }
}

/// Builds a single Newtonsoft Write switch arm for a variant in untagged mode.
///
/// No discriminator is written. Unit variants emit `WriteNull()`. Newtype
/// variants use `serializer.Serialize(writer, val.Value)`. Struct variants
/// write a flat object with their fields.
fn build_untagged_newtonsoft_write_arm(
    variant: &TaggedVariant,
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let csharp_name = &variant.csharp_name;
    let var_name = csharp_safe_var_name(&variant.csharp_name);

    match &variant.data {
        TaggedVariantData::Unit => {
            let arm = format!(
                "{case_indent}case {csharp_name}:\n\
                 {body_indent}writer.WriteNull();\n\
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
                         {bi}serializer.Serialize(writer, {var}.Value);\n\
                         {bi}break;",
                        ci = #case_indent,
                        name = #csharp_name,
                        var = #var_name,
                        bi = #body_indent,
                    )
                }
            }
        }
        TaggedVariantData::Struct(fields) => build_untagged_newtonsoft_write_struct_arm(
            csharp_name,
            &var_name,
            fields,
            case_indent,
            body_indent,
            config,
        ),
    }
}

/// Builds a Newtonsoft Write switch arm for a struct variant in untagged mode.
///
/// Writes a flat object containing just the struct fields, with no
/// discriminator or wrapping.
///
/// Handles flattened fields by dispatching on [`FlattenKind`]:
/// - `None`: regular compile-time property write
/// - `Struct`: runtime iteration over `csharp_fields()` properties
/// - `HashMap`: skipped (extension data handled by serializer)
fn build_untagged_newtonsoft_write_struct_arm(
    csharp_name: &str,
    var_name: &str,
    fields: &[CSharpField],
    case_indent: &str,
    body_indent: &str,
    config: &CSharpConfig,
) -> TokenStream {
    let field_exprs = build_write_field_exprs(fields, var_name, body_indent, config);

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
                case = #case_indent,
                name = #csharp_name,
                var = #var_name,
                body = #body_indent,
                fields = fields_block,
            )
        }
    }
}

/// Builds the final token stream for file-scoped namespace output (C# 10+).
#[expect(
    clippy::too_many_arguments,
    reason = "format template builder with orthogonal layout concerns"
)]
fn build_file_scoped(
    using_block: &str,
    namespace: &str,
    class_attrs: &str,
    csharp_name: &str,
    indent: &str,
    variant_exprs: &[TokenStream],
    converter_expr: Option<&TokenStream>,
    has_hashmap_flatten: bool,
    nullable_checks: &[TokenStream],
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

            let nullable_checks: Vec<bool> = vec![#(#nullable_checks),*];
            let nullable_directive = if #has_hashmap_flatten || nullable_checks.iter().any(|&x| x) {
                "#nullable enable\n"
            } else {
                ""
            };

            format!(
                "// <auto-generated/>\n\
                 {nullable}\
                 {using}\n\
                 \n\
                 namespace {ns};\n\
                 \n\
                 {attrs}\n\
                 {indent}public abstract record {name}{body}\n",
                nullable = nullable_directive,
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
#[expect(
    clippy::too_many_arguments,
    reason = "format template builder with orthogonal layout concerns"
)]
fn build_block_scoped(
    using_block: &str,
    namespace: &str,
    class_attrs: &str,
    csharp_name: &str,
    indent: &str,
    variant_exprs: &[TokenStream],
    converter_expr: Option<&TokenStream>,
    has_hashmap_flatten: bool,
    nullable_checks: &[TokenStream],
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

            let nullable_checks: Vec<bool> = vec![#(#nullable_checks),*];
            let nullable_directive = if #has_hashmap_flatten || nullable_checks.iter().any(|&x| x) {
                "#nullable enable\n"
            } else {
                ""
            };

            format!(
                "// <auto-generated/>\n\
                 {nullable}\
                 {using}\n\
                 \n\
                 namespace {ns}\n\
                 {{\n\
                 {indent}{attrs}\n\
                 {indent}public abstract record {name}{body}\n\
                 }}\n",
                nullable = nullable_directive,
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
