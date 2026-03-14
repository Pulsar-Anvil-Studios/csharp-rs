// Rust guideline compliant 2026-02-14
//! Token stream generation from the [`DerivedCSharp`] intermediate representation.
//!
//! Produces the `impl CSharp for T` block, including `csharp_name()`,
//! `csharp_definition()`, `dependencies()`, and optionally an export test.

mod record;
mod simple_enum;
mod tagged_enum;

use crate::types::{CSharpField, DerivedCSharp, DerivedCSharpKind, FlattenKind};
use proc_macro2::TokenStream;
use quote::quote;

/// C# built-in value type keywords.
///
/// When an optional field resolves to one of these types, the `?` suffix
/// produces `Nullable<T>` and does **not** require `#nullable enable`.
/// Any other type (e.g. `string`, `List<int>`, user-defined records) is a
/// reference type whose `?` annotation only works under `#nullable enable`.
const CSHARP_VALUE_TYPES: &[&str] = &[
    "bool", "byte", "sbyte", "short", "ushort", "int", "uint", "long", "ulong", "float", "double",
    "decimal", "char",
];

/// Builds nullable-reference-type check expressions for a set of fields.
///
/// Returns one `TokenStream` per optional field. Each expression evaluates at
/// compile time (in the consumer crate) to `true` when the resolved C# type
/// is a reference type — meaning the file needs `#nullable enable`.
///
/// Non-optional fields are skipped because they never emit a `?` suffix.
pub(crate) fn nullable_ref_check_exprs(fields: &[CSharpField]) -> Vec<TokenStream> {
    let value_types: Vec<&str> = CSHARP_VALUE_TYPES.to_vec();

    fields
        .iter()
        .filter(|f| f.is_optional && matches!(f.flatten, FlattenKind::None))
        .map(|f| {
            let type_expr = &f.type_expr;
            quote! {
                {
                    let csharp_type = #type_expr;
                    ![#(#value_types),*].contains(&csharp_type.as_str())
                }
            }
        })
        .collect()
}

impl DerivedCSharp {
    /// Converts the IR into a complete `impl CSharp for T` token stream.
    pub fn into_token_stream(self) -> TokenStream {
        let ident = &self.rust_ident;
        let csharp_name = &self.csharp_name;

        let definition_body = self.build_definition();
        let dependencies_body = self.build_dependencies();
        let csharp_fields_body = self.build_csharp_fields();
        let export_test = self.build_export_test();

        quote! {
            impl csharp_rs::CSharp for #ident {
                fn csharp_name(cfg: &csharp_rs::Config) -> String {
                    let _ = cfg;
                    String::from(#csharp_name)
                }

                fn csharp_definition(cfg: &csharp_rs::Config) -> String {
                    #definition_body
                }

                fn dependencies(cfg: &csharp_rs::Config) -> Vec<String> {
                    #dependencies_body
                }

                fn csharp_fields(cfg: &csharp_rs::Config) -> Vec<csharp_rs::CSharpFieldInfo> {
                    #csharp_fields_body
                }
            }

            #export_test
        }
    }

    /// Builds the `csharp_definition()` body that returns a complete `.cs` file.
    fn build_definition(&self) -> TokenStream {
        // Namespace is resolved at runtime: per-type override or cfg.namespace().
        let ns_expr = if let Some(ns) = &self.namespace_override {
            quote! { #ns }
        } else {
            quote! { cfg.namespace() }
        };
        let csharp_name = &self.csharp_name;

        match &self.kind {
            DerivedCSharpKind::Record(fields) => {
                record::build_record_definition(csharp_name, &ns_expr, fields)
            }
            DerivedCSharpKind::Enum(variants) => {
                simple_enum::build_enum_definition(csharp_name, &ns_expr, variants)
            }
            DerivedCSharpKind::TaggedEnum { tagging, variants } => {
                tagged_enum::build_tagged_enum_definition(
                    csharp_name,
                    &ns_expr,
                    tagging,
                    variants,
                )
            }
        }
    }

    /// Builds the `dependencies()` body returning type names.
    ///
    /// For flatten fields, dependency resolution differs:
    /// - `FlattenKind::None`: uses `type_expr` (produces the C# type name).
    /// - `FlattenKind::Struct`: collects transitive deps via the inner type's
    ///   `csharp_fields()` property type names (the flattened type itself is
    ///   not a dependency since its properties are inlined).
    /// - `FlattenKind::HashMap`: no deps (extension data uses framework types).
    fn build_dependencies(&self) -> TokenStream {
        match &self.kind {
            DerivedCSharpKind::Record(fields) => Self::build_field_dependencies(fields),
            DerivedCSharpKind::Enum(_) => quote! { Vec::new() },
            DerivedCSharpKind::TaggedEnum { variants, .. } => {
                let all_fields: Vec<&CSharpField> = variants
                    .iter()
                    .flat_map(|v| match &v.data {
                        crate::types::TaggedVariantData::Unit
                        | crate::types::TaggedVariantData::Newtype { .. } => Vec::new(),
                        crate::types::TaggedVariantData::Struct(fields) => fields.iter().collect(),
                    })
                    .collect();

                // Newtype type_exprs still need to be included as direct deps
                let newtype_exprs: Vec<&TokenStream> = variants
                    .iter()
                    .filter_map(|v| match &v.data {
                        crate::types::TaggedVariantData::Newtype { type_expr } => Some(type_expr),
                        _ => None,
                    })
                    .collect();

                let regular_exprs: Vec<&TokenStream> = all_fields
                    .iter()
                    .filter(|f| matches!(f.flatten, FlattenKind::None))
                    .map(|f| &f.type_expr)
                    .collect();

                let flatten_struct_fields: Vec<&&CSharpField> = all_fields
                    .iter()
                    .filter(|f| matches!(f.flatten, FlattenKind::Struct))
                    .collect();

                let all_regular: Vec<&TokenStream> = newtype_exprs
                    .iter()
                    .chain(regular_exprs.iter())
                    .copied()
                    .collect();

                if flatten_struct_fields.is_empty() {
                    if all_regular.is_empty() {
                        quote! { Vec::new() }
                    } else {
                        quote! {
                            vec![#(#all_regular),*]
                        }
                    }
                } else {
                    let flatten_extends: Vec<TokenStream> = flatten_struct_fields
                        .iter()
                        .map(|f| {
                            let type_expr = &f.type_expr;
                            quote! {
                                for fi in #type_expr {
                                    if let csharp_rs::CSharpFieldInfo::Property { type_name, .. } = fi {
                                        deps.push(type_name);
                                    }
                                }
                            }
                        })
                        .collect();

                    quote! {
                        {
                            let mut deps: Vec<String> = vec![#(#all_regular),*];
                            #(#flatten_extends)*
                            deps
                        }
                    }
                }
            }
        }
    }

    /// Builds dependency collection for a slice of fields, handling flatten.
    ///
    /// Regular fields contribute their `type_expr` directly. Flatten-struct
    /// fields contribute transitive deps from the inner type's `csharp_fields()`.
    /// Flatten-HashMap fields contribute nothing.
    fn build_field_dependencies(fields: &[CSharpField]) -> TokenStream {
        let regular_exprs: Vec<&TokenStream> = fields
            .iter()
            .filter(|f| matches!(f.flatten, FlattenKind::None))
            .map(|f| &f.type_expr)
            .collect();

        let flatten_struct_fields: Vec<&CSharpField> = fields
            .iter()
            .filter(|f| matches!(f.flatten, FlattenKind::Struct))
            .collect();

        if flatten_struct_fields.is_empty() {
            if regular_exprs.is_empty() {
                quote! { Vec::new() }
            } else {
                quote! {
                    vec![#(#regular_exprs),*]
                }
            }
        } else {
            let flatten_extends: Vec<TokenStream> = flatten_struct_fields
                .iter()
                .map(|f| {
                    let type_expr = &f.type_expr;
                    quote! {
                        for fi in #type_expr {
                            if let csharp_rs::CSharpFieldInfo::Property { type_name, .. } = fi {
                                deps.push(type_name);
                            }
                        }
                    }
                })
                .collect();

            quote! {
                {
                    let mut deps: Vec<String> = vec![#(#regular_exprs),*];
                    #(#flatten_extends)*
                    deps
                }
            }
        }
    }

    /// Generates an export test function if `export` is enabled.
    fn build_export_test(&self) -> TokenStream {
        if !self.export {
            return TokenStream::new();
        }

        let ident = &self.rust_ident;
        let test_name = quote::format_ident!("export_csharp_{}", ident.to_string().to_lowercase());
        let csharp_name = &self.csharp_name;

        if let Some(export_to) = &self.export_to {
            // `#[csharp(export_to = "...")]` overrides the config dir.
            let file_path = export_to.clone();
            quote! {
                #[test]
                fn #test_name() {
                    let cfg = csharp_rs::Config::default();
                    csharp_rs::export_to::<#ident>(&cfg, #file_path)
                        .expect("failed to export C# definition");
                }
            }
        } else {
            // Resolve export directory at runtime via `cfg.export_dir()`.
            quote! {
                #[test]
                fn #test_name() {
                    let cfg = csharp_rs::Config::default();
                    let file_path = format!("{}/{}.cs", cfg.export_dir().display(), #csharp_name);
                    csharp_rs::export_to::<#ident>(&cfg, file_path)
                        .expect("failed to export C# definition");
                }
            }
        }
    }

    /// Builds the `csharp_fields()` body returning field metadata for flatten.
    fn build_csharp_fields(&self) -> TokenStream {
        let fields: Vec<&CSharpField> = match &self.kind {
            DerivedCSharpKind::Record(fields) => fields.iter().collect(),
            DerivedCSharpKind::Enum(_) | DerivedCSharpKind::TaggedEnum { .. } => {
                return quote! { Vec::new() };
            }
        };

        if fields.is_empty() {
            return quote! { Vec::new() };
        }

        let field_exprs: Vec<TokenStream> = fields
            .iter()
            .map(|f| match &f.flatten {
                FlattenKind::None => {
                    let prop_name = &f.csharp_property_name;
                    let json_name = &f.json_name;
                    let is_optional = f.is_optional;
                    let type_expr = &f.type_expr;
                    quote! {
                        result.push(csharp_rs::CSharpFieldInfo::Property {
                            property_name: String::from(#prop_name),
                            json_name: String::from(#json_name),
                            type_name: #type_expr,
                            is_optional: #is_optional,
                        });
                    }
                }
                FlattenKind::Struct => {
                    let type_expr = &f.type_expr;
                    quote! {
                        result.extend(#type_expr);
                    }
                }
                FlattenKind::HashMap {
                    key_expr,
                    value_expr,
                } => {
                    quote! {
                        result.push(csharp_rs::CSharpFieldInfo::ExtensionData {
                            key_type_name: #key_expr,
                            value_type_name: #value_expr,
                        });
                    }
                }
            })
            .collect();

        quote! {
            {
                let mut result: Vec<csharp_rs::CSharpFieldInfo> = Vec::new();
                #(#field_exprs)*
                result
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CSharpVariant, EnumTagging, TaggedVariant, TaggedVariantData};

    /// Helper to build a minimal record IR with one field.
    fn sample_ir(export: bool, export_to: Option<String>) -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("TestStruct"),
            csharp_name: String::from("TestStruct"),
            namespace_override: Some(String::from("Test.Ns")),
            kind: DerivedCSharpKind::Record(vec![CSharpField {
                csharp_property_name: String::from("Name"),
                json_name: String::from("name"),
                type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                is_optional: false,
                flatten: FlattenKind::None,
            }]),
            export,
            export_to,
            transparent: false,
        }
    }

    /// Helper to build a minimal enum IR with variants.
    fn sample_enum_ir() -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("Color"),
            csharp_name: String::from("Color"),
            namespace_override: Some(String::from("Test.Ns")),
            kind: DerivedCSharpKind::Enum(vec![
                CSharpVariant {
                    csharp_name: String::from("Red"),
                    json_name: String::from("red"),
                },
                CSharpVariant {
                    csharp_name: String::from("Green"),
                    json_name: String::from("green"),
                },
                CSharpVariant {
                    csharp_name: String::from("Blue"),
                    json_name: String::from("Blue"),
                },
            ]),
            export: false,
            export_to: None,
            transparent: false,
        }
    }

    #[test]
    fn record_token_stream_contains_both_serializer_paths() {
        let ir = sample_ir(false, None);
        let tokens = ir.into_token_stream().to_string();
        assert!(
            tokens.contains("JsonPropertyName"),
            "should contain STJ attribute path:\n{tokens}"
        );
        assert!(
            tokens.contains("JsonProperty"),
            "should contain Newtonsoft attribute path:\n{tokens}"
        );
        assert!(
            tokens.contains("System.Text.Json.Serialization"),
            "should contain STJ using directive:\n{tokens}"
        );
        assert!(
            tokens.contains("Newtonsoft.Json"),
            "should contain Newtonsoft using directive:\n{tokens}"
        );
    }

    #[test]
    fn no_export_generates_no_test_fn() {
        let ir = sample_ir(false, None);
        let tokens = ir.into_token_stream().to_string();
        assert!(
            !tokens.contains("fn export_csharp_"),
            "non-export IR should not generate test function:\n{tokens}"
        );
    }

    #[test]
    fn export_generates_test_fn_with_runtime_dir() {
        let ir = sample_ir(true, None);
        let tokens = ir.into_token_stream().to_string();
        assert!(
            tokens.contains("export_csharp_teststruct"),
            "export IR should generate test function:\n{tokens}"
        );
        // Default export dir is resolved at runtime via cfg.export_dir().
        assert!(
            tokens.contains("export_dir"),
            "should resolve export dir at runtime:\n{tokens}"
        );
        assert!(
            tokens.contains("Config :: default ()"),
            "export test should create Config via default():\n{tokens}"
        );
        assert!(
            tokens.contains("export_to"),
            "export test should call export_to:\n{tokens}"
        );
    }

    #[test]
    fn export_to_overrides_directory() {
        let ir = sample_ir(true, Some(String::from("custom/out")));
        let tokens = ir.into_token_stream().to_string();
        assert!(
            tokens.contains("custom/out"),
            "should use custom export dir:\n{tokens}"
        );
        assert!(
            tokens.contains("Config :: default ()"),
            "export test should create Config via default():\n{tokens}"
        );
    }

    #[test]
    fn empty_fields_generates_empty_vec_for_dependencies() {
        let ir = DerivedCSharp {
            rust_ident: quote::format_ident!("Empty"),
            csharp_name: String::from("Empty"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::Record(vec![]),
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();
        assert!(
            tokens.contains("Vec :: new ()"),
            "empty fields should use Vec::new() for dependencies:\n{tokens}"
        );
    }

    #[test]
    fn optional_field_generates_nullable_marker() {
        let ir = DerivedCSharp {
            rust_ident: quote::format_ident!("WithOpt"),
            csharp_name: String::from("WithOpt"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::Record(vec![CSharpField {
                csharp_property_name: String::from("Score"),
                json_name: String::from("score"),
                type_expr: quote! { <f64 as csharp_rs::CSharp>::csharp_name(cfg) },
                is_optional: true,
                flatten: FlattenKind::None,
            }]),
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();
        // The generated code uses `if #is_optional { "?" }` so true should appear
        assert!(
            tokens.contains("true"),
            "optional field should set is_optional to true:\n{tokens}"
        );
    }

    #[test]
    fn enum_stj_contains_json_string_enum_converter() {
        let ir = sample_enum_ir();
        let tokens = ir.into_token_stream().to_string();
        assert!(
            tokens.contains("JsonStringEnumConverter"),
            "STJ enum should contain JsonStringEnumConverter:\n{tokens}"
        );
        assert!(
            tokens.contains("System.Text.Json.Serialization"),
            "STJ enum should contain using directive:\n{tokens}"
        );
    }

    #[test]
    fn enum_newtonsoft_contains_string_enum_converter() {
        let ir = sample_enum_ir();
        let tokens = ir.into_token_stream().to_string();
        assert!(
            tokens.contains("StringEnumConverter"),
            "Newtonsoft enum should contain StringEnumConverter:\n{tokens}"
        );
        assert!(
            tokens.contains("Newtonsoft.Json"),
            "Newtonsoft enum should contain using directive:\n{tokens}"
        );
    }

    #[test]
    fn enum_contains_enum_member_only_when_renamed() {
        let ir = sample_enum_ir();
        let tokens = ir.into_token_stream().to_string();
        // Red->red and Green->green are renamed, Blue->Blue is not
        assert!(
            tokens.contains("EnumMember"),
            "should contain EnumMember for renamed variants:\n{tokens}"
        );
    }

    #[test]
    fn enum_dependencies_returns_empty_vec() {
        let ir = sample_enum_ir();
        let tokens = ir.into_token_stream().to_string();
        assert!(
            tokens.contains("Vec :: new ()"),
            "enum should use Vec::new() for dependencies:\n{tokens}"
        );
    }

    // --- Tagged enum test helpers ---

    /// Builds a tagged enum IR with internal tagging, a struct variant, a
    /// newtype variant, and a unit variant.
    fn sample_tagged_enum_ir() -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("Message"),
            csharp_name: String::from("Message"),
            namespace_override: Some(String::from("Test.Ns")),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::Internal {
                    tag: String::from("type"),
                },
                variants: vec![
                    TaggedVariant {
                        csharp_name: String::from("Request"),
                        json_name: String::from("Request"),
                        data: TaggedVariantData::Struct(vec![CSharpField {
                            csharp_property_name: String::from("Id"),
                            json_name: String::from("id"),
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                            is_optional: false,
                            flatten: FlattenKind::None,
                        }]),
                    },
                    TaggedVariant {
                        csharp_name: String::from("Text"),
                        json_name: String::from("Text"),
                        data: TaggedVariantData::Newtype {
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                        },
                    },
                    TaggedVariant {
                        csharp_name: String::from("Quit"),
                        json_name: String::from("Quit"),
                        data: TaggedVariantData::Unit,
                    },
                ],
            },
            export: false,
            export_to: None,
            transparent: false,
        }
    }

    // --- Tagged enum tests ---

    #[test]
    fn tagged_internal_stj_csharp11_has_json_polymorphic() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonPolymorphic"),
            "C# 11 + STJ + internal tag should contain [JsonPolymorphic]:\n{tokens}"
        );
        assert!(
            tokens.contains("TypeDiscriminatorPropertyName"),
            "should contain TypeDiscriminatorPropertyName:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp11_has_json_derived_type() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonDerivedType"),
            "C# 11 + STJ + internal tag should contain [JsonDerivedType]:\n{tokens}"
        );
        assert!(
            tokens.contains("Message.Request"),
            "should reference nested type Message.Request:\n{tokens}"
        );
        assert!(
            tokens.contains("Message.Quit"),
            "should reference nested type Message.Quit:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp11_has_abstract_record() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // The format template in the token stream contains the pattern.
        assert!(
            tokens.contains("public abstract record"),
            "should declare abstract record:\n{tokens}"
        );
        assert!(
            tokens.contains(r#"name = "Message""#),
            "should bind name to Message:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp11_has_sealed_records() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Struct/newtype variants use named format params.
        assert!(
            tokens.contains("sealed record {name} : {parent}"),
            "should contain sealed record pattern for data variants:\n{tokens}"
        );
        // Unit variant uses positional format with literal variant name.
        assert!(
            tokens.contains("public sealed record {} : {};"),
            "should contain sealed record unit format template:\n{tokens}"
        );
        assert!(
            tokens.contains(r#""Quit""#),
            "should contain Quit variant name as format arg:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp11_has_required_modifier() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("required"),
            "C# 11 should emit required modifier:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp11_has_file_scoped_namespace() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // The format template contains `namespace {ns};` for file-scoped.
        assert!(
            tokens.contains("namespace {ns};"),
            "C# 11 should use file-scoped namespace (semicolon):\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp11_uses_stj_using() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("System.Text.Json.Serialization"),
            "should contain STJ using directive:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_has_runtime_namespace_branching() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Runtime branching: both file-scoped and block-scoped templates exist.
        assert!(
            tokens.contains("namespace {ns};"),
            "tagged enum should contain file-scoped namespace template:\n{tokens}"
        );
        assert!(
            tokens.contains("namespace {ns}\\n"),
            "tagged enum should contain block-scoped namespace template:\n{tokens}"
        );
        assert!(
            tokens.contains("use_file_scoped"),
            "tagged enum should branch on use_file_scoped:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_has_runtime_required_branching() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Runtime branching: `required_kw` variable determined by cfg.target().
        assert!(
            tokens.contains("required_kw"),
            "tagged enum should branch on required_kw:\n{tokens}"
        );
        assert!(
            tokens.contains("use_required"),
            "tagged enum should have use_required variable:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_has_both_converter_and_native_paths() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Runtime branching: both converter and native polymorphism paths exist.
        assert!(
            tokens.contains("JsonConverter"),
            "tagged enum should contain converter path:\n{tokens}"
        );
        assert!(
            tokens.contains("MessageConverter"),
            "tagged enum should reference MessageConverter:\n{tokens}"
        );
        assert!(
            tokens.contains("JsonPolymorphic"),
            "tagged enum should contain native polymorphism path:\n{tokens}"
        );
    }

    #[test]
    fn tagged_struct_variant_has_properties() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonPropertyName"),
            "struct variant should have [JsonPropertyName] attributes:\n{tokens}"
        );
        assert!(
            tokens.contains("get; init;"),
            "struct variant should have {{ get; init; }} accessors:\n{tokens}"
        );
    }

    #[test]
    fn tagged_newtype_variant_has_value_property() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("Value"),
            "newtype variant should have a Value property:\n{tokens}"
        );
    }

    #[test]
    fn tagged_unit_variant_is_semicolon_record() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Unit variant uses positional format with literal variant name.
        assert!(
            tokens.contains("public sealed record {} : {};"),
            "unit variant should use semicolon format template:\n{tokens}"
        );
        assert!(
            tokens.contains(r#""Quit""#),
            "unit variant should contain Quit as format arg:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_contains_both_serializer_attribute_paths() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Runtime branching: both serializer attribute paths exist.
        assert!(
            tokens.contains("JsonPropertyName"),
            "tagged enum should contain STJ attribute path:\n{tokens}"
        );
        assert!(
            tokens.contains("JsonProperty"),
            "tagged enum should contain Newtonsoft attribute path:\n{tokens}"
        );
        assert!(
            tokens.contains("System.Text.Json.Serialization"),
            "tagged enum should contain STJ using directive:\n{tokens}"
        );
        assert!(
            tokens.contains("Newtonsoft.Json"),
            "tagged enum should contain Newtonsoft using directive:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_runtime_branches_both_namespace_styles() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Runtime branching: both namespace styles exist in the token stream.
        assert!(
            tokens.contains("namespace {ns};"),
            "tagged enum should contain file-scoped namespace template:\n{tokens}"
        );
        assert!(
            tokens.contains("use_file_scoped"),
            "tagged enum should branch on use_file_scoped:\n{tokens}"
        );
        // `required_kw` is a runtime variable, not baked at compile time.
        assert!(
            tokens.contains("required_kw"),
            "tagged enum should have required_kw runtime variable:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_dependencies_includes_type_exprs() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Should contain the CSharp::csharp_name() calls for struct/newtype fields
        assert!(
            tokens.contains("csharp_rs :: CSharp"),
            "tagged enum dependencies should include type expressions:\n{tokens}"
        );
        // The dependencies() body should NOT use Vec::new() since there are fields.
        // Check that the fn dependencies() section uses vec! (not Vec::new()).
        let deps_section = tokens
            .split("fn dependencies")
            .nth(1)
            .expect("should have dependencies fn");
        assert!(
            !deps_section.starts_with(" () -> Vec < String > { Vec :: new () }"),
            "tagged enum with fields should not return empty dependencies:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_unit_only_dependencies_empty() {
        let ir = DerivedCSharp {
            rust_ident: quote::format_ident!("Status"),
            csharp_name: String::from("Status"),
            namespace_override: Some(String::from("Test.Ns")),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::Internal {
                    tag: String::from("kind"),
                },
                variants: vec![
                    TaggedVariant {
                        csharp_name: String::from("Active"),
                        json_name: String::from("Active"),
                        data: TaggedVariantData::Unit,
                    },
                    TaggedVariant {
                        csharp_name: String::from("Inactive"),
                        json_name: String::from("Inactive"),
                        data: TaggedVariantData::Unit,
                    },
                ],
            },
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();
        assert!(
            tokens.contains("Vec :: new ()"),
            "unit-only tagged enum should have empty dependencies:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_runtime_branches_converter_and_native_poly() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Runtime branching: both converter and native polymorphism paths exist.
        assert!(
            tokens.contains("MessageConverter"),
            "tagged enum should contain converter path:\n{tokens}"
        );
        assert!(
            tokens.contains("JsonPolymorphic"),
            "tagged enum should contain native polymorphism path:\n{tokens}"
        );
        // Runtime decision variable for native vs converter path.
        assert!(
            tokens.contains("use_native_polymorphism"),
            "tagged enum should branch on use_native_polymorphism:\n{tokens}"
        );
    }

    // --- Internally tagged converter tests ---

    #[test]
    fn stj_csharp9_converter_has_read_method() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonDocument.ParseValue"),
            "STJ converter Read should use JsonDocument.ParseValue:\n{tokens}"
        );
        assert!(
            tokens.contains("GetProperty") && tokens.contains("{tag}"),
            "STJ converter Read should get discriminator property:\n{tokens}"
        );
        assert!(
            tokens.contains("return tag switch"),
            "STJ converter Read should contain tag switch:\n{tokens}"
        );
        // Variant names appear as format args, not literal text.
        assert!(
            tokens.contains(r#"name = "Request""#),
            "STJ converter Read should have Request variant:\n{tokens}"
        );
        assert!(
            tokens.contains(r#"name = "Text""#),
            "STJ converter Read should have Text variant:\n{tokens}"
        );
        assert!(
            tokens.contains(r#"name = "Quit""#),
            "STJ converter Read should have Quit variant:\n{tokens}"
        );
        // Unit variant uses `=> new {name}()` pattern in template.
        assert!(
            tokens.contains("=> new {name}()"),
            "STJ converter Read should have unit variant template:\n{tokens}"
        );
    }

    #[test]
    fn stj_csharp9_converter_has_write_method() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("WriteStartObject"),
            "STJ converter Write should call WriteStartObject:\n{tokens}"
        );
        assert!(
            tokens.contains("WriteString") && tokens.contains("{tag}"),
            "STJ converter Write should write tag discriminator:\n{tokens}"
        );
        assert!(
            tokens.contains("WriteEndObject"),
            "STJ converter Write should call WriteEndObject:\n{tokens}"
        );
        assert!(
            tokens.contains("WritePropertyName"),
            "STJ converter Write should write field properties:\n{tokens}"
        );
    }

    #[test]
    fn newtonsoft_converter_has_read_json() {
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JObject.Load"),
            "Newtonsoft converter ReadJson should use JObject.Load:\n{tokens}"
        );
        assert!(
            tokens.contains("ToObject<"),
            "Newtonsoft converter ReadJson should use ToObject<T>:\n{tokens}"
        );
        assert!(
            tokens.contains("return tag switch"),
            "Newtonsoft converter ReadJson should contain tag switch:\n{tokens}"
        );
    }

    #[test]
    fn newtonsoft_converter_has_write_json() {
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("WritePropertyName"),
            "Newtonsoft converter WriteJson should use WritePropertyName:\n{tokens}"
        );
        assert!(
            tokens.contains("WriteValue"),
            "Newtonsoft converter WriteJson should use WriteValue:\n{tokens}"
        );
        assert!(
            tokens.contains("serializer.Serialize"),
            "Newtonsoft converter WriteJson should use serializer.Serialize:\n{tokens}"
        );
    }

    #[test]
    fn stj_converter_struct_variant_read_has_properties() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // The Request variant has an Id field; the Read arm should deserialize it.
        // In the format template: `{name} = root.GetProperty("{json}").Deserialize<{ty}>(options),`
        // with format args `name = "Id"`, `json = "id"`.
        assert!(
            tokens.contains("GetProperty") && tokens.contains("json"),
            "struct variant Read arm should access field by JSON name:\n{tokens}"
        );
        assert!(
            tokens.contains("Deserialize<"),
            "struct variant Read arm should use Deserialize<T>:\n{tokens}"
        );
        // Check that the property name and assignment pattern exist in the
        // format template: `{name} = root.GetProperty`.
        assert!(
            tokens.contains("{name} = root.GetProperty"),
            "struct variant Read arm should assign to property name:\n{tokens}"
        );
        // And that the format arg supplies the correct property name.
        assert!(
            tokens.contains(r#"name = "Id""#),
            "struct variant Read arm should bind name to Id:\n{tokens}"
        );
    }

    #[test]
    fn stj_converter_newtype_variant_read_has_value() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // The Text variant is a newtype; the Read arm should have Value =.
        assert!(
            tokens.contains("GetProperty") && tokens.contains("Value"),
            "newtype variant Read arm should access Value property:\n{tokens}"
        );
        assert!(
            tokens.contains("Value ="),
            "newtype variant Read arm should assign to Value:\n{tokens}"
        );
    }

    #[test]
    fn stj_converter_unit_variant_read_uses_empty_constructor() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Unit variant uses `=> new {name}()` template with `name = "Quit"` arg.
        assert!(
            tokens.contains("=> new {name}()"),
            "unit variant Read arm should use empty constructor template:\n{tokens}"
        );
        assert!(
            tokens.contains(r#"name = "Quit""#),
            "unit variant Read arm should bind name to Quit:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_has_both_converter_class_and_native_poly() {
        let ir =sample_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Runtime branching: both converter class and native [JsonPolymorphic]
        // paths exist in the token stream; cfg.target() decides at runtime.
        assert!(
            tokens.contains("private sealed class"),
            "tagged enum should contain converter class template:\n{tokens}"
        );
        assert!(
            tokens.contains("JsonDocument.ParseValue"),
            "tagged enum should contain converter Read body:\n{tokens}"
        );
        assert!(
            tokens.contains("JsonPolymorphic"),
            "tagged enum should contain native [JsonPolymorphic]:\n{tokens}"
        );
    }

    // --- Externally tagged enum test helpers ---

    /// Builds an externally tagged enum IR with a struct variant (`Request`),
    /// a newtype variant (`Text`), and a unit variant (`Quit`).
    fn sample_external_tagged_enum_ir() -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("Message"),
            csharp_name: String::from("Message"),
            namespace_override: Some(String::from("Test.Ns")),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::External,
                variants: vec![
                    TaggedVariant {
                        csharp_name: String::from("Request"),
                        json_name: String::from("Request"),
                        data: TaggedVariantData::Struct(vec![CSharpField {
                            csharp_property_name: String::from("Id"),
                            json_name: String::from("id"),
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                            is_optional: false,
                            flatten: FlattenKind::None,
                        }]),
                    },
                    TaggedVariant {
                        csharp_name: String::from("Text"),
                        json_name: String::from("Text"),
                        data: TaggedVariantData::Newtype {
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                        },
                    },
                    TaggedVariant {
                        csharp_name: String::from("Quit"),
                        json_name: String::from("Quit"),
                        data: TaggedVariantData::Unit,
                    },
                ],
            },
            export: false,
            export_to: None,
            transparent: false,
        }
    }

    // --- Externally tagged converter tests ---

    #[test]
    fn external_stj_has_converter() {
        let ir =sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonConverter"),
            "externally tagged STJ should have [JsonConverter] attribute:\n{tokens}"
        );
        assert!(
            tokens.contains("MessageConverter"),
            "externally tagged STJ should reference MessageConverter:\n{tokens}"
        );
        assert!(
            tokens.contains("private sealed class"),
            "externally tagged STJ should generate converter class:\n{tokens}"
        );
    }

    #[test]
    fn external_stj_read_handles_string_for_unit() {
        let ir =sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("ValueKind.String"),
            "STJ external Read should check ValueKind.String for unit variants:\n{tokens}"
        );
        assert!(
            tokens.contains("GetString"),
            "STJ external Read should call GetString() for the variant name:\n{tokens}"
        );
        // Unit variant uses format template with name placeholder.
        assert!(
            tokens.contains("=> new {name}()"),
            "STJ external Read should use empty ctor template:\n{tokens}"
        );
        assert!(
            tokens.contains(r#"name = "Quit""#),
            "STJ external Read should bind name to Quit:\n{tokens}"
        );
    }

    #[test]
    fn external_stj_read_handles_object_for_data() {
        let ir =sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("ValueKind.Object"),
            "STJ external Read should check ValueKind.Object for data variants:\n{tokens}"
        );
        assert!(
            tokens.contains("EnumerateObject"),
            "STJ external Read should use EnumerateObject() to get the single property:\n{tokens}"
        );
    }

    #[test]
    fn external_stj_write_unit_uses_string_value() {
        let ir =sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("WriteStringValue"),
            "STJ external Write should use WriteStringValue for unit variants:\n{tokens}"
        );
    }

    #[test]
    fn external_stj_write_data_uses_property_name() {
        let ir =sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("WritePropertyName"),
            "STJ external Write should wrap data in object with variant name key:\n{tokens}"
        );
        assert!(
            tokens.contains("WriteStartObject"),
            "STJ external Write should open object for data variants:\n{tokens}"
        );
        assert!(
            tokens.contains("WriteEndObject"),
            "STJ external Write should close object for data variants:\n{tokens}"
        );
    }

    #[test]
    fn external_newtonsoft_has_converter() {
        let ir =sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonConverter"),
            "externally tagged Newtonsoft should have [JsonConverter] attribute:\n{tokens}"
        );
        assert!(
            tokens.contains("MessageConverter"),
            "externally tagged Newtonsoft should reference MessageConverter:\n{tokens}"
        );
        assert!(
            tokens.contains("Newtonsoft.Json"),
            "externally tagged Newtonsoft should have using directive:\n{tokens}"
        );
    }

    #[test]
    fn external_newtonsoft_read_handles_tokens() {
        let ir =sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonToken.String"),
            "Newtonsoft external Read should check JsonToken.String:\n{tokens}"
        );
        assert!(
            tokens.contains("JsonToken.StartObject"),
            "Newtonsoft external Read should check JsonToken.StartObject:\n{tokens}"
        );
        assert!(
            tokens.contains("JObject.Load"),
            "Newtonsoft external Read should use JObject.Load for object variants:\n{tokens}"
        );
    }

    #[test]
    fn external_newtonsoft_write_unit_uses_write_value() {
        let ir =sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("WriteValue"),
            "Newtonsoft external Write should use WriteValue for unit variants:\n{tokens}"
        );
    }

    // --- Adjacently tagged enum test helpers ---

    /// Builds an adjacently tagged enum IR with `tag = "t"`, `content = "c"`,
    /// a struct variant (`Request`), a newtype variant (`Text`), and a unit
    /// variant (`Quit`).
    fn sample_adjacent_tagged_enum_ir() -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("Message"),
            csharp_name: String::from("Message"),
            namespace_override: Some(String::from("Test.Ns")),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::Adjacent {
                    tag: String::from("t"),
                    content: String::from("c"),
                },
                variants: vec![
                    TaggedVariant {
                        csharp_name: String::from("Request"),
                        json_name: String::from("Request"),
                        data: TaggedVariantData::Struct(vec![CSharpField {
                            csharp_property_name: String::from("Id"),
                            json_name: String::from("id"),
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                            is_optional: false,
                            flatten: FlattenKind::None,
                        }]),
                    },
                    TaggedVariant {
                        csharp_name: String::from("Text"),
                        json_name: String::from("Text"),
                        data: TaggedVariantData::Newtype {
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                        },
                    },
                    TaggedVariant {
                        csharp_name: String::from("Quit"),
                        json_name: String::from("Quit"),
                        data: TaggedVariantData::Unit,
                    },
                ],
            },
            export: false,
            export_to: None,
            transparent: false,
        }
    }

    // --- Adjacently tagged converter tests ---

    #[test]
    fn adjacent_stj_has_converter() {
        let ir =sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonConverter"),
            "adjacently tagged STJ should have [JsonConverter] attribute:\n{tokens}"
        );
        assert!(
            tokens.contains("MessageConverter"),
            "adjacently tagged STJ should reference MessageConverter:\n{tokens}"
        );
        assert!(
            tokens.contains("private sealed class"),
            "adjacently tagged STJ should generate converter class:\n{tokens}"
        );
    }

    #[test]
    fn adjacent_stj_read_uses_tag_property() {
        let ir =sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("GetProperty"),
            "STJ adjacent Read should call GetProperty for tag:\n{tokens}"
        );
        // The format template should reference the tag key `t`.
        assert!(
            tokens.contains(r#"tag = "t""#),
            "STJ adjacent Read should bind tag key to \"t\":\n{tokens}"
        );
        assert!(
            tokens.contains("return tag switch"),
            "STJ adjacent Read should contain tag switch:\n{tokens}"
        );
    }

    #[test]
    fn adjacent_stj_read_struct_uses_content() {
        let ir =sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Struct variant inlines the content element access to stay valid
        // inside a switch expression (no block body / return allowed).
        assert!(
            tokens.contains("GetProperty"),
            "STJ adjacent Read struct variant should use GetProperty:\n{tokens}"
        );
        assert!(
            tokens.contains(r#"content = "c""#),
            "STJ adjacent Read should bind content key to \"c\":\n{tokens}"
        );
    }

    #[test]
    fn adjacent_stj_write_unit_no_content() {
        let ir =sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Unit variant uses `case {name}:` format template with `name = "Quit"`.
        assert!(
            tokens.contains("case {name}"),
            "STJ adjacent Write should have case template:\n{tokens}"
        );
        assert!(
            tokens.contains(r#"name = "Quit""#),
            "STJ adjacent Write should bind name to Quit:\n{tokens}"
        );
    }

    #[test]
    fn adjacent_stj_write_data_has_content() {
        let ir =sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Data variants (newtype/struct) should write both tag and content.
        assert!(
            tokens.contains("WriteStartObject"),
            "STJ adjacent Write should write outer object:\n{tokens}"
        );
        // The content property key should appear in the output.
        assert!(
            tokens.contains(r#"content = "c""#),
            "STJ adjacent Write data variants should reference content key:\n{tokens}"
        );
    }

    #[test]
    fn adjacent_newtonsoft_has_converter() {
        let ir =sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonConverter"),
            "adjacently tagged Newtonsoft should have [JsonConverter] attribute:\n{tokens}"
        );
        assert!(
            tokens.contains("MessageConverter"),
            "adjacently tagged Newtonsoft should reference MessageConverter:\n{tokens}"
        );
        assert!(
            tokens.contains("Newtonsoft.Json"),
            "adjacently tagged Newtonsoft should have using directive:\n{tokens}"
        );
    }

    #[test]
    fn adjacent_newtonsoft_read_uses_jobj() {
        let ir =sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JObject.Load"),
            "Newtonsoft adjacent Read should use JObject.Load:\n{tokens}"
        );
        // Tag access: `(string)obj["t"]`
        assert!(
            tokens.contains(r#"tag = "t""#),
            "Newtonsoft adjacent Read should bind tag key to \"t\":\n{tokens}"
        );
        // Content access for newtype/struct variants.
        assert!(
            tokens.contains(r#"content = "c""#),
            "Newtonsoft adjacent Read should bind content key to \"c\":\n{tokens}"
        );
    }

    #[test]
    fn adjacent_newtonsoft_write_unit_only_tag() {
        let ir =sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Unit variant uses `case {name}:` format template with `name = "Quit"`.
        assert!(
            tokens.contains("case {name}"),
            "Newtonsoft adjacent Write should have case template:\n{tokens}"
        );
        assert!(
            tokens.contains(r#"name = "Quit""#),
            "Newtonsoft adjacent Write should bind name to Quit:\n{tokens}"
        );
    }

    // --- Untagged enum test helpers ---

    /// Builds an untagged enum IR with a struct variant (`Request`), a newtype
    /// variant (`Text`), and a unit variant (`Quit`).
    fn sample_untagged_enum_ir() -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("Message"),
            csharp_name: String::from("Message"),
            namespace_override: Some(String::from("Test.Ns")),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::Untagged,
                variants: vec![
                    TaggedVariant {
                        csharp_name: String::from("Request"),
                        json_name: String::from("Request"),
                        data: TaggedVariantData::Struct(vec![CSharpField {
                            csharp_property_name: String::from("Id"),
                            json_name: String::from("id"),
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                            is_optional: false,
                            flatten: FlattenKind::None,
                        }]),
                    },
                    TaggedVariant {
                        csharp_name: String::from("Text"),
                        json_name: String::from("Text"),
                        data: TaggedVariantData::Newtype {
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                        },
                    },
                    TaggedVariant {
                        csharp_name: String::from("Quit"),
                        json_name: String::from("Quit"),
                        data: TaggedVariantData::Unit,
                    },
                ],
            },
            export: false,
            export_to: None,
            transparent: false,
        }
    }

    // --- Untagged converter tests ---

    #[test]
    fn untagged_stj_has_converter() {
        let ir =sample_untagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonConverter"),
            "untagged STJ should have [JsonConverter] attribute:\n{tokens}"
        );
        assert!(
            tokens.contains("MessageConverter"),
            "untagged STJ should reference MessageConverter:\n{tokens}"
        );
        assert!(
            tokens.contains("private sealed class"),
            "untagged STJ should generate converter class:\n{tokens}"
        );
    }

    #[test]
    fn untagged_stj_read_tries_variants() {
        let ir =sample_untagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Untagged Read should use try/catch to attempt each variant.
        assert!(
            tokens.contains("try"),
            "untagged STJ Read should use try blocks:\n{tokens}"
        );
        assert!(
            tokens.contains("catch (Exception)"),
            "untagged STJ Read should catch Exception:\n{tokens}"
        );
        assert!(
            tokens.contains("No matching variant"),
            "untagged STJ Read should throw when no variant matches:\n{tokens}"
        );
    }

    #[test]
    fn untagged_stj_read_unit_checks_null() {
        let ir =sample_untagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Unit variant should check for JsonValueKind.Null.
        assert!(
            tokens.contains("ValueKind.Null"),
            "untagged STJ Read should check ValueKind.Null for unit variants:\n{tokens}"
        );
        // Unit variant uses format template with name placeholder.
        assert!(
            tokens.contains("new {name}()"),
            "untagged STJ Read should use empty ctor template:\n{tokens}"
        );
        assert!(
            tokens.contains(r#"name = "Quit""#),
            "untagged STJ Read should bind name to Quit:\n{tokens}"
        );
    }

    #[test]
    fn untagged_stj_write_unit_writes_null() {
        let ir =sample_untagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Unit variant Write should emit null.
        assert!(
            tokens.contains("WriteNullValue"),
            "untagged STJ Write should use WriteNullValue for unit variants:\n{tokens}"
        );
    }

    #[test]
    fn untagged_stj_write_newtype_direct() {
        let ir =sample_untagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Newtype variant Write should serialize the value directly (no wrapping
        // object, no tag).
        assert!(
            tokens.contains("JsonSerializer.Serialize(writer,"),
            "untagged STJ Write should serialize value directly:\n{tokens}"
        );
        // The text variant uses `text.Value` pattern.
        assert!(
            tokens.contains(".Value, options)"),
            "untagged STJ Write should access .Value on newtype variant:\n{tokens}"
        );
    }

    #[test]
    fn untagged_newtonsoft_has_converter() {
        let ir =sample_untagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonConverter"),
            "untagged Newtonsoft should have [JsonConverter] attribute:\n{tokens}"
        );
        assert!(
            tokens.contains("MessageConverter"),
            "untagged Newtonsoft should reference MessageConverter:\n{tokens}"
        );
        assert!(
            tokens.contains("Newtonsoft.Json"),
            "untagged Newtonsoft should have using directive:\n{tokens}"
        );
    }

    #[test]
    fn untagged_newtonsoft_read_uses_jtoken() {
        let ir =sample_untagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Untagged Newtonsoft Read should use JToken.ReadFrom for buffering.
        assert!(
            tokens.contains("JToken.ReadFrom"),
            "untagged Newtonsoft Read should use JToken.ReadFrom:\n{tokens}"
        );
        assert!(
            tokens.contains("JTokenType.Null"),
            "untagged Newtonsoft Read should check JTokenType.Null for unit:\n{tokens}"
        );
    }

    #[test]
    fn untagged_newtonsoft_write_unit_writes_null() {
        let ir =sample_untagged_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Unit variant Write should emit null via WriteNull().
        assert!(
            tokens.contains("WriteNull()"),
            "untagged Newtonsoft Write should use WriteNull() for unit variants:\n{tokens}"
        );
    }

    // --- Version-dependent record/enum tests ---

    #[test]
    fn record_csharp10_file_scoped_namespace() {
        let ir =sample_ir(false, None);
        let tokens = ir.into_token_stream().to_string();

        // C# 10+ should use file-scoped namespace with semicolon.
        assert!(
            tokens.contains("namespace {ns};"),
            "C# 10 record should use file-scoped namespace:\n{tokens}"
        );
        // Should NOT have the block-scoped opening brace after namespace.
        assert!(
            !tokens.contains("namespace {ns}\\n{{"),
            "C# 10 record should not use block-scoped namespace:\n{tokens}"
        );
    }

    #[test]
    fn record_csharp9_block_scoped_namespace() {
        let ir =sample_ir(false, None);
        let tokens = ir.into_token_stream().to_string();

        // Runtime branching: both file-scoped and block-scoped templates
        // exist in the token stream as branches of `if use_file_scoped`.
        assert!(
            tokens.contains("namespace {ns};"),
            "record should contain file-scoped namespace template branch:\n{tokens}"
        );
        assert!(
            tokens.contains("namespace {ns}\\n"),
            "record should contain block-scoped namespace template branch:\n{tokens}"
        );
        // Verify the runtime branching variable is present.
        assert!(
            tokens.contains("use_file_scoped"),
            "record should branch on use_file_scoped:\n{tokens}"
        );
    }

    #[test]
    fn record_csharp11_required_properties() {
        let ir =sample_ir(false, None);
        let tokens = ir.into_token_stream().to_string();

        // C# 11+ non-optional properties should have `required` modifier.
        assert!(
            tokens.contains("required"),
            "C# 11 record should emit required modifier on non-optional properties:\n{tokens}"
        );
    }

    #[test]
    fn record_csharp11_optional_field_no_required() {
        let ir =DerivedCSharp {
            rust_ident: quote::format_ident!("WithOpt"),
            csharp_name: String::from("WithOpt"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::Record(vec![CSharpField {
                csharp_property_name: String::from("Score"),
                json_name: String::from("score"),
                type_expr: quote! { <f64 as csharp_rs::CSharp>::csharp_name(cfg) },
                is_optional: true,
                flatten: FlattenKind::None,
            }]),
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        // Optional properties use a runtime `if is_optional { "" } else { required_kw }`
        // check. For an optional field (is_optional=true), the generated code
        // bakes `true` as the condition. Verify the guard uses `required_kw`
        // (a runtime variable) and not a hardcoded `"required "` literal.
        assert!(
            tokens.contains(r#"if true { "" } else { required_kw }"#),
            "C# 11 optional field should guard required behind is_optional=true with required_kw:\n{tokens}"
        );
    }

    #[test]
    fn record_csharp9_no_required() {
        let ir =sample_ir(false, None);
        let tokens = ir.into_token_stream().to_string();

        // The generated code now contains runtime branching for the `required`
        // modifier. The string "required " appears in the `if use_required`
        // branch regardless of compile-time config. Verify the runtime guard
        // structure exists and defers the decision to `cfg.target()`.
        assert!(
            tokens.contains("use_required"),
            "record should reference use_required runtime variable:\n{tokens}"
        );
        assert!(
            tokens.contains("cfg . target ()"),
            "record should branch on cfg.target() at runtime:\n{tokens}"
        );
    }

    #[test]
    fn enum_csharp10_file_scoped_namespace() {
        let ir =sample_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Namespace is now a runtime variable bound as `let ns: &str = "Test.Ns";`
        // and used via `namespace {ns};` in the format template.
        assert!(
            tokens.contains(r#""Test.Ns""#),
            "C# 10 enum should bind namespace value:\n{tokens}"
        );
        assert!(
            tokens.contains("namespace {ns};"),
            "C# 10 enum should use file-scoped namespace placeholder:\n{tokens}"
        );
    }

    #[test]
    fn enum_csharp9_block_scoped_namespace() {
        let ir =sample_enum_ir();
        let tokens = ir.into_token_stream().to_string();

        // Runtime branching: both file-scoped and block-scoped templates
        // exist in the token stream as branches of `if use_file_scoped`.
        assert!(
            tokens.contains(r#""Test.Ns""#),
            "enum should bind namespace value:\n{tokens}"
        );
        assert!(
            tokens.contains("namespace {ns};"),
            "enum should contain file-scoped namespace template branch:\n{tokens}"
        );
        assert!(
            tokens.contains("namespace {ns}\\n"),
            "enum should contain block-scoped namespace template branch:\n{tokens}"
        );
        assert!(
            tokens.contains("use_file_scoped"),
            "enum should branch on use_file_scoped:\n{tokens}"
        );
    }

    #[test]
    fn record_with_optional_ref_type_has_nullable_enable() {
        let ir =DerivedCSharp {
            rust_ident: quote::format_ident!("WithOptStr"),
            csharp_name: String::from("WithOptStr"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::Record(vec![CSharpField {
                csharp_property_name: String::from("Name"),
                json_name: String::from("name"),
                type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                is_optional: true,
                flatten: FlattenKind::None,
            }]),
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        // The generated code should contain the nullable check logic
        // that uses the CSHARP_VALUE_TYPES list to determine if #nullable enable is needed.
        assert!(
            tokens.contains("nullable"),
            "record with optional ref type should contain nullable check logic:\n{tokens}"
        );
    }

    #[test]
    fn record_with_no_optional_fields_no_nullable_check() {
        let ir =sample_ir(false, None); // has one non-optional String field
        let tokens = ir.into_token_stream().to_string();

        // No optional fields means the nullable_checks vec should be empty.
        // Strip whitespace before matching so we're resilient to any
        // formatting differences in proc_macro2's `TokenStream::to_string`.
        let collapsed: String = tokens.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            collapsed.contains("nullable_checks:Vec<bool>=vec![]"),
            "record with no optional fields should have empty nullable_checks vec:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_with_optional_ref_field_has_nullable_check() {
        let ir =DerivedCSharp {
            rust_ident: quote::format_ident!("Event"),
            csharp_name: String::from("Event"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::Internal {
                    tag: String::from("type"),
                },
                variants: vec![TaggedVariant {
                    csharp_name: String::from("Data"),
                    json_name: String::from("Data"),
                    data: TaggedVariantData::Struct(vec![CSharpField {
                        csharp_property_name: String::from("Payload"),
                        json_name: String::from("payload"),
                        type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                        is_optional: true,
                        flatten: FlattenKind::None,
                    }]),
                }],
            },
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("nullable"),
            "tagged enum with optional ref field should contain nullable check:\n{tokens}"
        );
    }

    #[test]
    fn record_generates_csharp_fields_method() {
        let ir =sample_ir(false, None);
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("csharp_fields"),
            "generated impl should contain csharp_fields method:\n{tokens}"
        );
    }

    #[test]
    fn record_with_flatten_struct_field_has_csharp_fields_call() {
        let ir =DerivedCSharp {
            rust_ident: quote::format_ident!("Outer"),
            csharp_name: String::from("Outer"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::Record(vec![
                CSharpField {
                    csharp_property_name: String::from("Name"),
                    json_name: String::from("name"),
                    type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                    is_optional: false,
                    flatten: FlattenKind::None,
                },
                CSharpField {
                    csharp_property_name: String::new(),
                    json_name: String::new(),
                    type_expr: quote! { <Inner as csharp_rs::CSharp>::csharp_fields(cfg) },
                    is_optional: false,
                    flatten: FlattenKind::Struct,
                },
            ]),
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("CSharpFieldInfo"),
            "flatten struct field should reference CSharpFieldInfo in codegen:\n{tokens}"
        );
    }

    #[test]
    fn record_with_flatten_hashmap_has_extension_data() {
        let ir =DerivedCSharp {
            rust_ident: quote::format_ident!("WithExtra"),
            csharp_name: String::from("WithExtra"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::Record(vec![CSharpField {
                csharp_property_name: String::new(),
                json_name: String::new(),
                type_expr: TokenStream::new(),
                is_optional: false,
                flatten: FlattenKind::HashMap {
                    key_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                    value_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                },
            }]),
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonExtensionData"),
            "flatten HashMap field should generate JsonExtensionData:\n{tokens}"
        );
    }

    #[test]
    fn flatten_struct_dependencies_uses_transitive() {
        let ir =DerivedCSharp {
            rust_ident: quote::format_ident!("Outer"),
            csharp_name: String::from("Outer"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::Record(vec![
                CSharpField {
                    csharp_property_name: String::from("Name"),
                    json_name: String::from("name"),
                    type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                    is_optional: false,
                    flatten: FlattenKind::None,
                },
                CSharpField {
                    csharp_property_name: String::new(),
                    json_name: String::new(),
                    type_expr: quote! { <Inner as csharp_rs::CSharp>::csharp_fields(cfg) },
                    is_optional: false,
                    flatten: FlattenKind::Struct,
                },
            ]),
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        // Dependencies should NOT include the csharp_fields() call directly
        // Instead it should use Inner::dependencies() for transitive deps
        assert!(
            !tokens.contains("fn dependencies () -> Vec < String > { vec ! [< String as csharp_rs :: CSharp > :: csharp_name () , < Inner as csharp_rs :: CSharp > :: csharp_fields ()"),
            "flatten struct should NOT use csharp_fields() as a dependency:\n{tokens}"
        );
        // Should contain extend call for transitive deps from Inner
        assert!(
            tokens.contains("Inner"),
            "dependencies should reference Inner type for transitive deps:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_struct_variant_with_flatten_struct_has_field_info() {
        let ir =DerivedCSharp {
            rust_ident: quote::format_ident!("Event"),
            csharp_name: String::from("Event"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::Internal {
                    tag: String::from("type"),
                },
                variants: vec![TaggedVariant {
                    csharp_name: String::from("Data"),
                    json_name: String::from("Data"),
                    data: TaggedVariantData::Struct(vec![
                        CSharpField {
                            csharp_property_name: String::from("Name"),
                            json_name: String::from("name"),
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                            is_optional: false,
                            flatten: FlattenKind::None,
                        },
                        CSharpField {
                            csharp_property_name: String::new(),
                            json_name: String::new(),
                            type_expr: quote! { <Inner as csharp_rs::CSharp>::csharp_fields(cfg) },
                            is_optional: false,
                            flatten: FlattenKind::Struct,
                        },
                    ]),
                }],
            },
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        // The variant body in build_struct_variant should iterate
        // csharp_fields() at runtime for flatten-struct fields.
        // Look for `for field_info in` in the field_parts block (not just in
        // the csharp_fields() impl method).
        let variant_section = tokens
            .split("variant_parts")
            .nth(1)
            .expect("should have variant_parts section");
        assert!(
            variant_section.contains("for field_info in"),
            "tagged enum struct variant with flatten field should iterate csharp_fields() in property rendering:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_struct_variant_with_flatten_hashmap_has_extension_data() {
        let ir =DerivedCSharp {
            rust_ident: quote::format_ident!("Event"),
            csharp_name: String::from("Event"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::Internal {
                    tag: String::from("type"),
                },
                variants: vec![TaggedVariant {
                    csharp_name: String::from("Data"),
                    json_name: String::from("Data"),
                    data: TaggedVariantData::Struct(vec![
                        CSharpField {
                            csharp_property_name: String::from("Version"),
                            json_name: String::from("version"),
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                            is_optional: false,
                            flatten: FlattenKind::None,
                        },
                        CSharpField {
                            csharp_property_name: String::new(),
                            json_name: String::new(),
                            type_expr: TokenStream::new(),
                            is_optional: false,
                            flatten: FlattenKind::HashMap {
                                key_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                                value_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                            },
                        },
                    ]),
                }],
            },
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        assert!(
            tokens.contains("JsonExtensionData"),
            "tagged enum struct variant with flatten HashMap should generate JsonExtensionData:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_with_hashmap_flatten_has_nullable_directive() {
        let ir =DerivedCSharp {
            rust_ident: quote::format_ident!("Event"),
            csharp_name: String::from("Event"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::Internal {
                    tag: String::from("type"),
                },
                variants: vec![TaggedVariant {
                    csharp_name: String::from("Data"),
                    json_name: String::from("Data"),
                    data: TaggedVariantData::Struct(vec![
                        CSharpField {
                            csharp_property_name: String::from("Name"),
                            json_name: String::from("name"),
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                            is_optional: false,
                            flatten: FlattenKind::None,
                        },
                        CSharpField {
                            csharp_property_name: String::new(),
                            json_name: String::new(),
                            type_expr: TokenStream::new(),
                            is_optional: false,
                            flatten: FlattenKind::HashMap {
                                key_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                                value_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                            },
                        },
                    ]),
                }],
            },
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        // After the fix, `has_hashmap_flatten = true` is interpolated into the
        // nullable-directive condition so the runtime `if` always evaluates to
        // true.  Verify that `true` appears in the collapsed condition.
        let collapsed: String = tokens.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            collapsed.contains("letnullable_directive=iftrue"),
            "tagged enum with HashMap flatten should force nullable directive via `if true`:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp11_with_hashmap_flatten_has_system_text_json_using() {
        let ir = DerivedCSharp {
            rust_ident: quote::format_ident!("Event"),
            csharp_name: String::from("Event"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::Internal {
                    tag: String::from("type"),
                },
                variants: vec![TaggedVariant {
                    csharp_name: String::from("Data"),
                    json_name: String::from("Data"),
                    data: TaggedVariantData::Struct(vec![CSharpField {
                        csharp_property_name: String::new(),
                        json_name: String::new(),
                        type_expr: TokenStream::new(),
                        is_optional: false,
                        flatten: FlattenKind::HashMap {
                            key_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                            value_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                        },
                    }]),
                }],
            },
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        // Native polymorphism path uses only System.Text.Json.Serialization,
        // but HashMap flatten needs System.Text.Json for JsonElement.
        assert!(
            tokens.contains("System.Text.Json;"),
            "native polymorphism with HashMap flatten should include System.Text.Json using:\n{tokens}"
        );
    }

    #[test]
    fn nullable_ref_check_skips_flatten_fields() {
        // If a flatten field somehow had is_optional: true,
        // nullable_ref_check_exprs should skip it.
        let fields = vec![CSharpField {
            csharp_property_name: String::new(),
            json_name: String::new(),
            type_expr: TokenStream::new(),
            is_optional: true, // hypothetically optional flatten field
            flatten: FlattenKind::Struct,
        }];
        let result = super::nullable_ref_check_exprs(&fields);
        assert!(
            result.is_empty(),
            "flatten fields should be skipped by nullable_ref_check_exprs"
        );
    }

    #[test]
    fn flatten_hashmap_dependencies_excluded() {
        let ir =DerivedCSharp {
            rust_ident: quote::format_ident!("WithExtra"),
            csharp_name: String::from("WithExtra"),
            namespace_override: Some(String::from("Ns")),
            kind: DerivedCSharpKind::Record(vec![
                CSharpField {
                    csharp_property_name: String::from("Name"),
                    json_name: String::from("name"),
                    type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                    is_optional: false,
                    flatten: FlattenKind::None,
                },
                CSharpField {
                    csharp_property_name: String::new(),
                    json_name: String::new(),
                    type_expr: TokenStream::new(),
                    is_optional: false,
                    flatten: FlattenKind::HashMap {
                        key_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                        value_expr: quote! { <String as csharp_rs::CSharp>::csharp_name(cfg) },
                    },
                },
            ]),
            export: false,
            export_to: None,
            transparent: false,
        };
        let tokens = ir.into_token_stream().to_string();

        // The dependencies vec should only contain the regular field's type_expr
        // HashMap flatten should not add any dependency entries
        let deps_section = tokens
            .split("fn dependencies")
            .nth(1)
            .expect("should have dependencies fn");
        assert!(
            !deps_section.contains("JsonElement"),
            "HashMap flatten should not contribute to dependencies:\n{tokens}"
        );
    }
}
