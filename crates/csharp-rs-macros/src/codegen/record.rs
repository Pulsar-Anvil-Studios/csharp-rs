// Rust guideline compliant 2026-02-15
//! Record (sealed record) code generation from struct fields.

use crate::types::{CSharpField, FlattenKind};
use proc_macro2::TokenStream;
use quote::quote;

/// Builds a `sealed record` definition token stream from struct fields.
///
/// The returned token stream evaluates at runtime to produce the complete
/// C# file contents including using directives, namespace, and property
/// declarations with JSON serializer attributes.
///
/// All configuration decisions (serializer library, C# version) are deferred
/// to runtime via `cfg: &csharp_rs::Config`, which is in scope in the
/// generated `csharp_definition(cfg)` method body.
///
/// Version-dependent features:
/// - **C# 10+**: file-scoped namespace (`namespace X;` instead of block).
/// - **C# 11+**: `required` modifier on non-optional properties.
pub fn build_record_definition(
    csharp_name: &str,
    ns_expr: &TokenStream,
    fields: &[CSharpField],
) -> TokenStream {
    let has_hashmap_flatten = fields
        .iter()
        .any(|f| matches!(f.flatten, FlattenKind::HashMap { .. }));

    let field_exprs = build_field_exprs(fields);
    let nullable_checks = super::nullable_ref_check_exprs(fields);

    // HashMap flatten fields always contribute nullable (extension data is reference type).
    let flatten_nullable = fields
        .iter()
        .any(|f| matches!(f.flatten, FlattenKind::HashMap { .. }));

    build_definition_body(
        ns_expr,
        csharp_name,
        &field_exprs,
        &nullable_checks,
        flatten_nullable,
        has_hashmap_flatten,
    )
}

/// Builds token stream statements for each record property.
///
/// Each expression is a block that pushes one or more `String` values onto
/// a pre-existing `field_parts: Vec<String>`. Regular fields push one entry,
/// flatten-struct fields push entries from the inner type's `csharp_fields()`,
/// and flatten-HashMap fields push a `[JsonExtensionData]` property.
///
/// Serializer-dependent values (`attr_name`, `extension_data_type`) and
/// version-dependent values (`required`, `prop_indent`) are resolved at
/// runtime via variables already in scope from the enclosing generated block.
fn build_field_exprs(fields: &[CSharpField]) -> Vec<TokenStream> {
    fields
        .iter()
        .map(|f| {
            match &f.flatten {
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
                                "{indent}[{attr}(\"{json}\")]\n{indent}public {req}{ty}{null} {name} {{ get; init; }}\n",
                                indent = prop_indent,
                                attr = attr_name,
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
                                    let req = if is_optional { "" } else { required_kw };
                                    field_parts.push(format!(
                                        "{indent}[{attr}(\"{json}\")]\n{indent}public {req}{ty}{null} {name} {{ get; init; }}\n",
                                        indent = prop_indent,
                                        attr = attr_name,
                                        json = json_name,
                                        req = req,
                                        ty = type_name,
                                        null = nullable,
                                        name = property_name,
                                    ));
                                }
                                csharp_rs::CSharpFieldInfo::ExtensionData { .. } => {
                                    field_parts.push(format!(
                                        "{indent}[JsonExtensionData]\n{indent}public {ext_type}? ExtensionData {{ get; set; }}\n",
                                        indent = prop_indent,
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
                            "{indent}[JsonExtensionData]\n{indent}public {ext_type}? ExtensionData {{ get; set; }}\n",
                            indent = prop_indent,
                            ext_type = extension_data_type,
                        ));
                    }
                }
            }
        })
        .collect()
}

/// Builds the complete definition body with runtime branching for serializer
/// and C# version.
///
/// The generated code resolves `cfg.serializer()` and `cfg.target()` at
/// runtime to select using directives, attribute names, namespace style,
/// and the `required` modifier.
fn build_definition_body(
    ns_expr: &TokenStream,
    csharp_name: &str,
    field_exprs: &[TokenStream],
    nullable_checks: &[TokenStream],
    flatten_nullable: bool,
    has_hashmap_flatten: bool,
) -> TokenStream {
    quote! {
        {
            let ns: &str = #ns_expr;

            // Runtime serializer selection
            let using_directive = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => "using System.Text.Json.Serialization;",
                csharp_rs::Serializer::Newtonsoft => "using Newtonsoft.Json;",
            };
            let extra_using = if #has_hashmap_flatten {
                match cfg.serializer() {
                    csharp_rs::Serializer::SystemTextJson => "\nusing System.Text.Json;",
                    csharp_rs::Serializer::Newtonsoft => "\nusing Newtonsoft.Json.Linq;",
                }
            } else {
                ""
            };
            let attr_name = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => "JsonPropertyName",
                csharp_rs::Serializer::Newtonsoft => "JsonProperty",
            };
            let extension_data_type = match cfg.serializer() {
                csharp_rs::Serializer::SystemTextJson => "Dictionary<string, JsonElement>",
                csharp_rs::Serializer::Newtonsoft => "Dictionary<string, JToken>",
            };

            // Runtime version selection
            let use_file_scoped = cfg.target() >= csharp_rs::CSharpVersion::CSharp10;
            let use_required = cfg.target() >= csharp_rs::CSharpVersion::CSharp11;
            let required_kw = if use_required { "required " } else { "" };
            let prop_indent = if use_file_scoped { "    " } else { "        " };

            // Build field parts
            let mut field_parts: Vec<String> = Vec::new();
            #(#field_exprs)*

            let mut fields = String::new();
            for (i, part) in field_parts.iter().enumerate() {
                if i > 0 {
                    fields.push('\n');
                }
                fields.push_str(part);
            }

            let nullable_checks: Vec<bool> = vec![#(#nullable_checks),*];
            let nullable_directive = if #flatten_nullable || nullable_checks.iter().any(|&x| x) {
                "#nullable enable\n"
            } else {
                ""
            };

            if use_file_scoped {
                format!(
                    concat!(
                        "// <auto-generated/>\n",
                        "{nullable}",
                        "{using}{extra_using}\n",
                        "\n",
                        "namespace {ns};\n",
                        "\n",
                        "public sealed record {name}\n",
                        "{{\n",
                        "{fields}",
                        "}}\n",
                    ),
                    nullable = nullable_directive,
                    using = using_directive,
                    extra_using = extra_using,
                    ns = ns,
                    name = #csharp_name,
                    fields = fields,
                )
            } else {
                format!(
                    concat!(
                        "// <auto-generated/>\n",
                        "{nullable}",
                        "{using}{extra_using}\n",
                        "\n",
                        "namespace {ns}\n",
                        "{{\n",
                        "    public sealed record {name}\n",
                        "    {{\n",
                        "{fields}",
                        "    }}\n",
                        "}}\n",
                    ),
                    nullable = nullable_directive,
                    using = using_directive,
                    extra_using = extra_using,
                    ns = ns,
                    name = #csharp_name,
                    fields = fields,
                )
            }
        }
    }
}
