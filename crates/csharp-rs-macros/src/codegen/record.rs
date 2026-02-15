// Rust guideline compliant 2026-02-14
//! Record (sealed record) code generation from struct fields.

use crate::config::{CSharpConfig, CSharpVersion, Serializer};
use crate::types::{CSharpField, FlattenKind};
use proc_macro2::TokenStream;
use quote::quote;

/// Builds a `sealed record` definition token stream from struct fields.
///
/// The returned token stream evaluates at compile time to produce the complete
/// C# file contents including using directives, namespace, and property
/// declarations with JSON serializer attributes.
///
/// Version-dependent features:
/// - **C# 10+**: file-scoped namespace (`namespace X;` instead of block).
/// - **C# 11+**: `required` modifier on non-optional properties.
pub fn build_record_definition(
    csharp_name: &str,
    ns_expr: &TokenStream,
    fields: &[CSharpField],
    config: &CSharpConfig,
) -> TokenStream {
    let use_file_scoped_ns = config.target >= CSharpVersion::CSharp10;
    let use_required = config.target >= CSharpVersion::CSharp11;

    let using_directive = match config.serializer {
        Serializer::SystemTextJson => "using System.Text.Json.Serialization;",
        Serializer::Newtonsoft => "using Newtonsoft.Json;",
    };

    let has_hashmap_flatten = fields
        .iter()
        .any(|f| matches!(f.flatten, FlattenKind::HashMap { .. }));

    let extra_using = if has_hashmap_flatten {
        match config.serializer {
            Serializer::SystemTextJson => "\nusing System.Text.Json;",
            Serializer::Newtonsoft => "\nusing Newtonsoft.Json.Linq;",
        }
    } else {
        ""
    };

    let prop_indent = if use_file_scoped_ns {
        "    "
    } else {
        "        "
    };

    let field_exprs = build_field_exprs(fields, config, prop_indent, use_required);
    let nullable_checks = super::nullable_ref_check_exprs(fields);

    // HashMap flatten fields always contribute nullable (extension data is reference type).
    let flatten_nullable = fields
        .iter()
        .any(|f| matches!(f.flatten, FlattenKind::HashMap { .. }));

    if use_file_scoped_ns {
        build_file_scoped(
            using_directive,
            extra_using,
            ns_expr,
            csharp_name,
            &field_exprs,
            &nullable_checks,
            flatten_nullable,
        )
    } else {
        build_block_scoped(
            using_directive,
            extra_using,
            ns_expr,
            csharp_name,
            &field_exprs,
            &nullable_checks,
            flatten_nullable,
        )
    }
}

/// Builds token stream statements for each record property.
///
/// Each expression is a block that pushes one or more `String` values onto
/// a pre-existing `field_parts: Vec<String>`. Regular fields push one entry,
/// flatten-struct fields push entries from the inner type's `csharp_fields()`,
/// and flatten-HashMap fields push a `[JsonExtensionData]` property.
fn build_field_exprs(
    fields: &[CSharpField],
    config: &CSharpConfig,
    prop_indent: &str,
    use_required: bool,
) -> Vec<TokenStream> {
    let attr_name = match config.serializer {
        Serializer::SystemTextJson => "JsonPropertyName",
        Serializer::Newtonsoft => "JsonProperty",
    };
    let required_kw = if use_required { "required " } else { "" };

    let extension_data_type = match config.serializer {
        Serializer::SystemTextJson => "Dictionary<string, JsonElement>",
        Serializer::Newtonsoft => "Dictionary<string, JToken>",
    };

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
                            let req = if #is_optional { "" } else { #required_kw };
                            format!(
                                "{indent}[{attr}(\"{json}\")]\n{indent}public {req}{ty}{null} {name} {{ get; init; }}\n",
                                indent = #prop_indent,
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
                                        "{indent}[{attr}(\"{json}\")]\n{indent}public {req}{ty}{null} {name} {{ get; init; }}\n",
                                        indent = #prop_indent,
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
                                        "{indent}[JsonExtensionData]\n{indent}public {ext_type}? ExtensionData {{ get; set; }}\n",
                                        indent = #prop_indent,
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
                            "{indent}[JsonExtensionData]\n{indent}public {ext_type}? ExtensionData {{ get; set; }}\n",
                            indent = #prop_indent,
                            ext_type = #extension_data_type,
                        ));
                    }
                }
            }
        })
        .collect()
}

/// Builds the final token stream using file-scoped namespace (C# 10+).
fn build_file_scoped(
    using_directive: &str,
    extra_using: &str,
    ns_expr: &TokenStream,
    csharp_name: &str,
    field_exprs: &[TokenStream],
    nullable_checks: &[TokenStream],
    flatten_nullable: bool,
) -> TokenStream {
    quote! {
        {
            let ns: &str = #ns_expr;
            let mut fields = String::new();
            let mut field_parts: Vec<String> = Vec::new();
            #(#field_exprs)*
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
                using = #using_directive,
                extra_using = #extra_using,
                ns = ns,
                name = #csharp_name,
                fields = fields,
            )
        }
    }
}

/// Builds the final token stream using block-scoped namespace (C# 9).
fn build_block_scoped(
    using_directive: &str,
    extra_using: &str,
    ns_expr: &TokenStream,
    csharp_name: &str,
    field_exprs: &[TokenStream],
    nullable_checks: &[TokenStream],
    flatten_nullable: bool,
) -> TokenStream {
    quote! {
        {
            let ns: &str = #ns_expr;
            let mut fields = String::new();
            let mut field_parts: Vec<String> = Vec::new();
            #(#field_exprs)*
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
                using = #using_directive,
                extra_using = #extra_using,
                ns = ns,
                name = #csharp_name,
                fields = fields,
            )
        }
    }
}
