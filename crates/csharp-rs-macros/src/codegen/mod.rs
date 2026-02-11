// Rust guideline compliant 2026-02-11
//! Token stream generation from the [`DerivedCSharp`] intermediate representation.
//!
//! Produces the `impl CSharp for T` block, including `csharp_name()`,
//! `csharp_definition()`, `dependencies()`, and optionally an export test.

mod record;
mod simple_enum;
mod tagged_enum;

use crate::config::CSharpConfig;
use crate::types::{DerivedCSharp, DerivedCSharpKind};
use proc_macro2::TokenStream;
use quote::quote;

impl DerivedCSharp {
    /// Converts the IR into a complete `impl CSharp for T` token stream.
    pub fn into_token_stream(self, config: &CSharpConfig) -> TokenStream {
        let ident = &self.rust_ident;
        let csharp_name = &self.csharp_name;

        let definition_body = self.build_definition(config);
        let dependencies_body = self.build_dependencies();
        let export_test = self.build_export_test(config);

        quote! {
            impl csharp_rs::CSharp for #ident {
                fn csharp_name() -> String {
                    String::from(#csharp_name)
                }

                fn csharp_definition() -> String {
                    #definition_body
                }

                fn dependencies() -> Vec<String> {
                    #dependencies_body
                }
            }

            #export_test
        }
    }

    /// Builds the `csharp_definition()` body that returns a complete `.cs` file.
    fn build_definition(&self, config: &CSharpConfig) -> TokenStream {
        let namespace: &str = self.namespace.as_ref();
        let csharp_name = &self.csharp_name;

        match &self.kind {
            DerivedCSharpKind::Record(fields) => {
                record::build_record_definition(csharp_name, namespace, fields, config)
            }
            DerivedCSharpKind::Enum(variants) => {
                simple_enum::build_enum_definition(csharp_name, namespace, variants, config)
            }
            DerivedCSharpKind::TaggedEnum { tagging, variants } => {
                tagged_enum::build_tagged_enum_definition(
                    csharp_name,
                    namespace,
                    tagging,
                    variants,
                    config,
                )
            }
        }
    }

    /// Builds the `dependencies()` body returning type names.
    fn build_dependencies(&self) -> TokenStream {
        match &self.kind {
            DerivedCSharpKind::Record(fields) => {
                let type_exprs: Vec<&TokenStream> = fields.iter().map(|f| &f.type_expr).collect();

                if type_exprs.is_empty() {
                    quote! { Vec::new() }
                } else {
                    quote! {
                        vec![#(#type_exprs),*]
                    }
                }
            }
            DerivedCSharpKind::Enum(_) => quote! { Vec::new() },
            DerivedCSharpKind::TaggedEnum { .. } => todo!("tagged enum codegen \u{2014} Task 5"),
        }
    }

    /// Generates an export test function if `export` is enabled.
    fn build_export_test(&self, config: &CSharpConfig) -> TokenStream {
        if !self.export {
            return TokenStream::new();
        }

        let ident = &self.rust_ident;
        let test_name = quote::format_ident!("export_csharp_{}", ident.to_string().to_lowercase());
        let csharp_name = &self.csharp_name;

        let export_dir = self
            .export_to
            .as_ref()
            .map_or_else(|| config.export_dir.display().to_string(), Clone::clone);

        let file_path = format!("{export_dir}/{csharp_name}.cs");

        quote! {
            #[test]
            fn #test_name() {
                csharp_rs::export_to::<#ident>(#file_path)
                    .expect("failed to export C# definition");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CSharpNamespace, Serializer};
    use crate::types::{CSharpField, CSharpVariant};
    use std::path::PathBuf;

    /// Helper to build a minimal record IR with one field.
    fn sample_ir(export: bool, export_to: Option<String>) -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("TestStruct"),
            csharp_name: String::from("TestStruct"),
            namespace: CSharpNamespace::new("Test.Ns").unwrap(),
            kind: DerivedCSharpKind::Record(vec![CSharpField {
                csharp_property_name: String::from("Name"),
                json_name: String::from("name"),
                type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
                is_optional: false,
            }]),
            export,
            export_to,
        }
    }

    /// Helper to build a minimal enum IR with variants.
    fn sample_enum_ir() -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("Color"),
            csharp_name: String::from("Color"),
            namespace: CSharpNamespace::new("Test.Ns").unwrap(),
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
        }
    }

    fn stj_config() -> CSharpConfig {
        CSharpConfig {
            serializer: Serializer::SystemTextJson,
            ..CSharpConfig::default()
        }
    }

    fn newtonsoft_config() -> CSharpConfig {
        CSharpConfig {
            serializer: Serializer::Newtonsoft,
            ..CSharpConfig::default()
        }
    }

    #[test]
    fn stj_token_stream_contains_json_property_name() {
        let ir = sample_ir(false, None);
        let tokens = ir.into_token_stream(&stj_config()).to_string();
        assert!(
            tokens.contains("JsonPropertyName"),
            "STJ output should contain JsonPropertyName:\n{tokens}"
        );
        assert!(
            tokens.contains("System.Text.Json.Serialization"),
            "STJ output should contain using directive:\n{tokens}"
        );
    }

    #[test]
    fn newtonsoft_token_stream_contains_json_property() {
        let ir = sample_ir(false, None);
        let tokens = ir.into_token_stream(&newtonsoft_config()).to_string();
        assert!(
            tokens.contains("JsonProperty"),
            "Newtonsoft output should contain JsonProperty:\n{tokens}"
        );
        assert!(
            tokens.contains("Newtonsoft.Json"),
            "Newtonsoft output should contain using directive:\n{tokens}"
        );
    }

    #[test]
    fn no_export_generates_no_test_fn() {
        let ir = sample_ir(false, None);
        let tokens = ir.into_token_stream(&stj_config()).to_string();
        assert!(
            !tokens.contains("fn export_csharp_"),
            "non-export IR should not generate test function:\n{tokens}"
        );
    }

    #[test]
    fn export_generates_test_fn_with_default_dir() {
        let ir = sample_ir(true, None);
        let config = stj_config();
        let tokens = ir.into_token_stream(&config).to_string();
        assert!(
            tokens.contains("export_csharp_teststruct"),
            "export IR should generate test function:\n{tokens}"
        );
        assert!(
            tokens.contains("csharp-bindings"),
            "should use default export dir:\n{tokens}"
        );
    }

    #[test]
    fn export_to_overrides_directory() {
        let ir = sample_ir(true, Some(String::from("custom/out")));
        let tokens = ir.into_token_stream(&stj_config()).to_string();
        assert!(
            tokens.contains("custom/out"),
            "should use custom export dir:\n{tokens}"
        );
    }

    #[test]
    fn empty_fields_generates_empty_vec_for_dependencies() {
        let ir = DerivedCSharp {
            rust_ident: quote::format_ident!("Empty"),
            csharp_name: String::from("Empty"),
            namespace: CSharpNamespace::new("Ns").unwrap(),
            kind: DerivedCSharpKind::Record(vec![]),
            export: false,
            export_to: None,
        };
        let tokens = ir.into_token_stream(&stj_config()).to_string();
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
            namespace: CSharpNamespace::new("Ns").unwrap(),
            kind: DerivedCSharpKind::Record(vec![CSharpField {
                csharp_property_name: String::from("Score"),
                json_name: String::from("score"),
                type_expr: quote! { <f64 as csharp_rs::CSharp>::csharp_name() },
                is_optional: true,
            }]),
            export: false,
            export_to: None,
        };
        let tokens = ir.into_token_stream(&stj_config()).to_string();
        // The generated code uses `if #is_optional { "?" }` so true should appear
        assert!(
            tokens.contains("true"),
            "optional field should set is_optional to true:\n{tokens}"
        );
    }

    #[test]
    fn export_test_uses_custom_export_dir_from_config() {
        let ir = sample_ir(true, None);
        let config = CSharpConfig {
            export_dir: PathBuf::from("my/export/path"),
            ..stj_config()
        };
        let tokens = ir.into_token_stream(&config).to_string();
        assert!(
            tokens.contains("my/export/path"),
            "should use config export dir:\n{tokens}"
        );
    }

    #[test]
    fn enum_stj_contains_json_string_enum_converter() {
        let ir = sample_enum_ir();
        let tokens = ir.into_token_stream(&stj_config()).to_string();
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
        let tokens = ir.into_token_stream(&newtonsoft_config()).to_string();
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
        let tokens = ir.into_token_stream(&stj_config()).to_string();
        // Red->red and Green->green are renamed, Blue->Blue is not
        assert!(
            tokens.contains("EnumMember"),
            "should contain EnumMember for renamed variants:\n{tokens}"
        );
    }

    #[test]
    fn enum_dependencies_returns_empty_vec() {
        let ir = sample_enum_ir();
        let tokens = ir.into_token_stream(&stj_config()).to_string();
        assert!(
            tokens.contains("Vec :: new ()"),
            "enum should use Vec::new() for dependencies:\n{tokens}"
        );
    }
}
