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
            DerivedCSharpKind::TaggedEnum { variants, .. } => {
                let type_exprs: Vec<&TokenStream> = variants
                    .iter()
                    .flat_map(|v| match &v.data {
                        crate::types::TaggedVariantData::Unit => Vec::new(),
                        crate::types::TaggedVariantData::Newtype { type_expr } => vec![type_expr],
                        crate::types::TaggedVariantData::Struct(fields) => {
                            fields.iter().map(|f| &f.type_expr).collect()
                        }
                    })
                    .collect();

                if type_exprs.is_empty() {
                    quote! { Vec::new() }
                } else {
                    quote! {
                        vec![#(#type_exprs),*]
                    }
                }
            }
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
    use crate::config::{CSharpNamespace, CSharpVersion, Serializer};
    use crate::types::{CSharpField, CSharpVariant, EnumTagging, TaggedVariant, TaggedVariantData};
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

    // --- Tagged enum test helpers ---

    /// Builds a tagged enum IR with internal tagging, a struct variant, a
    /// newtype variant, and a unit variant.
    fn sample_tagged_enum_ir() -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("Message"),
            csharp_name: String::from("Message"),
            namespace: CSharpNamespace::new("Test.Ns").unwrap(),
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
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
                            is_optional: false,
                        }]),
                    },
                    TaggedVariant {
                        csharp_name: String::from("Text"),
                        json_name: String::from("Text"),
                        data: TaggedVariantData::Newtype {
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
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
        }
    }

    fn stj_csharp11_config() -> CSharpConfig {
        CSharpConfig {
            serializer: Serializer::SystemTextJson,
            target: CSharpVersion::CSharp11,
            ..CSharpConfig::default()
        }
    }

    fn stj_csharp10_config() -> CSharpConfig {
        CSharpConfig {
            serializer: Serializer::SystemTextJson,
            target: CSharpVersion::CSharp10,
            ..CSharpConfig::default()
        }
    }

    // --- Tagged enum tests ---

    #[test]
    fn tagged_internal_stj_csharp11_has_json_polymorphic() {
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // Struct/newtype variants use format params; check the template pattern.
        assert!(
            tokens.contains("sealed record {name} : {parent}"),
            "should contain sealed record pattern for data variants:\n{tokens}"
        );
        // Unit variant is a full string literal.
        assert!(
            tokens.contains("sealed record Quit : Message;"),
            "should contain sealed record literal for unit variant:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp11_has_required_modifier() {
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        assert!(
            tokens.contains("required"),
            "C# 11 should emit required modifier:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp11_has_file_scoped_namespace() {
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // The format template contains `namespace {ns};` for file-scoped.
        assert!(
            tokens.contains("namespace {ns};"),
            "C# 11 should use file-scoped namespace (semicolon):\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp11_uses_stj_using() {
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        assert!(
            tokens.contains("System.Text.Json.Serialization"),
            "should contain STJ using directive:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp9_has_block_namespace() {
        let config = stj_config(); // default is C# 9
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // Block namespace should NOT have semicolon directly after `{ns}`.
        assert!(
            !tokens.contains("namespace {ns};"),
            "C# 9 should use block-scoped namespace, not file-scoped:\n{tokens}"
        );
        assert!(
            tokens.contains("namespace {ns}"),
            "should contain namespace placeholder:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp9_no_required() {
        let config = stj_config(); // default is C# 9
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        assert!(
            !tokens.contains("required"),
            "C# 9 should NOT emit required modifier:\n{tokens}"
        );
    }

    #[test]
    fn tagged_internal_stj_csharp9_has_json_converter() {
        let config = stj_config(); // default is C# 9
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        assert!(
            tokens.contains("JsonConverter"),
            "C# 9 + STJ should use [JsonConverter] (converter path):\n{tokens}"
        );
        assert!(
            tokens.contains("MessageConverter"),
            "should reference MessageConverter:\n{tokens}"
        );
        // Should NOT have [JsonPolymorphic] on C# 9
        assert!(
            !tokens.contains("JsonPolymorphic"),
            "C# 9 should NOT use [JsonPolymorphic]:\n{tokens}"
        );
    }

    #[test]
    fn tagged_struct_variant_has_properties() {
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        assert!(
            tokens.contains("Value"),
            "newtype variant should have a Value property:\n{tokens}"
        );
    }

    #[test]
    fn tagged_unit_variant_is_semicolon_record() {
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // Unit variant is a literal string in the token stream.
        assert!(
            tokens.contains("sealed record Quit : Message;"),
            "unit variant should use semicolon (no braces):\n{tokens}"
        );
    }

    #[test]
    fn tagged_newtonsoft_has_json_property() {
        let config = CSharpConfig {
            serializer: Serializer::Newtonsoft,
            target: CSharpVersion::CSharp11,
            ..CSharpConfig::default()
        };
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        assert!(
            tokens.contains("JsonProperty"),
            "Newtonsoft should use [JsonProperty] instead of [JsonPropertyName]:\n{tokens}"
        );
        assert!(
            !tokens.contains("JsonPropertyName"),
            "Newtonsoft should NOT contain JsonPropertyName:\n{tokens}"
        );
        assert!(
            tokens.contains("Newtonsoft.Json"),
            "Newtonsoft should contain using Newtonsoft.Json:\n{tokens}"
        );
    }

    #[test]
    fn tagged_csharp10_has_file_scoped_but_no_required() {
        let config = stj_csharp10_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // C# 10 uses converter path (not native polymorphism), so format template
        // should contain file-scoped namespace pattern.
        assert!(
            tokens.contains("namespace {ns};"),
            "C# 10 should use file-scoped namespace:\n{tokens}"
        );
        assert!(
            !tokens.contains("required"),
            "C# 10 should NOT emit required modifier:\n{tokens}"
        );
    }

    #[test]
    fn tagged_enum_dependencies_includes_type_exprs() {
        let config = stj_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
            namespace: CSharpNamespace::new("Test.Ns").unwrap(),
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
        };
        let tokens = ir.into_token_stream(&stj_config()).to_string();
        assert!(
            tokens.contains("Vec :: new ()"),
            "unit-only tagged enum should have empty dependencies:\n{tokens}"
        );
    }

    #[test]
    fn tagged_newtonsoft_uses_converter_path_not_polymorphic() {
        // Even with C# 11+, Newtonsoft should use converter path
        let config = CSharpConfig {
            serializer: Serializer::Newtonsoft,
            target: CSharpVersion::CSharp12,
            ..CSharpConfig::default()
        };
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        assert!(
            tokens.contains("MessageConverter"),
            "Newtonsoft should use converter path:\n{tokens}"
        );
        assert!(
            !tokens.contains("JsonPolymorphic"),
            "Newtonsoft should NOT use [JsonPolymorphic]:\n{tokens}"
        );
    }

    // --- Internally tagged converter tests ---

    #[test]
    fn stj_csharp9_converter_has_read_method() {
        let config = stj_config(); // C# 9 default
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        // Switch arms for each variant.
        assert!(
            tokens.contains("Request") && tokens.contains("=> new"),
            "STJ converter Read should have Request arm:\n{tokens}"
        );
        assert!(
            tokens.contains("Text") && tokens.contains("=> new"),
            "STJ converter Read should have Text arm:\n{tokens}"
        );
        assert!(
            tokens.contains("new Quit()"),
            "STJ converter Read should have Quit arm:\n{tokens}"
        );
    }

    #[test]
    fn stj_csharp9_converter_has_write_method() {
        let config = stj_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = CSharpConfig {
            serializer: Serializer::Newtonsoft,
            ..CSharpConfig::default()
        };
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = CSharpConfig {
            serializer: Serializer::Newtonsoft,
            ..CSharpConfig::default()
        };
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // The Quit variant is a unit; should use `new Quit()`.
        assert!(
            tokens.contains("new Quit()"),
            "unit variant Read arm should use empty constructor:\n{tokens}"
        );
    }

    #[test]
    fn stj_csharp11_has_no_converter_class() {
        let config = stj_csharp11_config();
        let ir = sample_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // C# 11 + STJ uses native [JsonPolymorphic], so no converter class.
        assert!(
            !tokens.contains("private sealed class"),
            "C# 11 + STJ should NOT generate converter class:\n{tokens}"
        );
        assert!(
            !tokens.contains("JsonDocument.ParseValue"),
            "C# 11 + STJ should NOT contain converter Read body:\n{tokens}"
        );
        // Should have the native attributes instead.
        assert!(
            tokens.contains("JsonPolymorphic"),
            "C# 11 + STJ should use native [JsonPolymorphic]:\n{tokens}"
        );
    }

    // --- Externally tagged enum test helpers ---

    /// Builds an externally tagged enum IR with a struct variant (`Request`),
    /// a newtype variant (`Text`), and a unit variant (`Quit`).
    fn sample_external_tagged_enum_ir() -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("Message"),
            csharp_name: String::from("Message"),
            namespace: CSharpNamespace::new("Test.Ns").unwrap(),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::External,
                variants: vec![
                    TaggedVariant {
                        csharp_name: String::from("Request"),
                        json_name: String::from("Request"),
                        data: TaggedVariantData::Struct(vec![CSharpField {
                            csharp_property_name: String::from("Id"),
                            json_name: String::from("id"),
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
                            is_optional: false,
                        }]),
                    },
                    TaggedVariant {
                        csharp_name: String::from("Text"),
                        json_name: String::from("Text"),
                        data: TaggedVariantData::Newtype {
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
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
        }
    }

    // --- Externally tagged converter tests ---

    #[test]
    fn external_stj_has_converter() {
        let config = stj_config();
        let ir = sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_config();
        let ir = sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        assert!(
            tokens.contains("ValueKind.String"),
            "STJ external Read should check ValueKind.String for unit variants:\n{tokens}"
        );
        assert!(
            tokens.contains("GetString"),
            "STJ external Read should call GetString() for the variant name:\n{tokens}"
        );
        assert!(
            tokens.contains("new Quit()"),
            "STJ external Read should construct unit variant with empty ctor:\n{tokens}"
        );
    }

    #[test]
    fn external_stj_read_handles_object_for_data() {
        let config = stj_config();
        let ir = sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_config();
        let ir = sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        assert!(
            tokens.contains("WriteStringValue"),
            "STJ external Write should use WriteStringValue for unit variants:\n{tokens}"
        );
    }

    #[test]
    fn external_stj_write_data_uses_property_name() {
        let config = stj_config();
        let ir = sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = newtonsoft_config();
        let ir = sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = newtonsoft_config();
        let ir = sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = newtonsoft_config();
        let ir = sample_external_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
            namespace: CSharpNamespace::new("Test.Ns").unwrap(),
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
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
                            is_optional: false,
                        }]),
                    },
                    TaggedVariant {
                        csharp_name: String::from("Text"),
                        json_name: String::from("Text"),
                        data: TaggedVariantData::Newtype {
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
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
        }
    }

    // --- Adjacently tagged converter tests ---

    #[test]
    fn adjacent_stj_has_converter() {
        let config = stj_config();
        let ir = sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_config();
        let ir = sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_config();
        let ir = sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // Struct variant should access content element.
        assert!(
            tokens.contains("contentElement"),
            "STJ adjacent Read struct variant should use contentElement:\n{tokens}"
        );
        assert!(
            tokens.contains(r#"content = "c""#),
            "STJ adjacent Read should bind content key to \"c\":\n{tokens}"
        );
    }

    #[test]
    fn adjacent_stj_write_unit_no_content() {
        let config = stj_config();
        let ir = sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // The unit variant Write arm should have WriteString for tag but NOT
        // contain the content key in the same case block. We check the unit
        // arm specifically: `case Quit:` followed by WriteStartObject,
        // WriteString (tag), WriteEndObject, break — no content property.
        assert!(
            tokens.contains("case Quit"),
            "STJ adjacent Write should have case Quit:\n{tokens}"
        );

        // Extract the Quit case block to verify no content key.
        let quit_section = tokens
            .split("case Quit")
            .nth(1)
            .and_then(|s| s.split("break;").next())
            .unwrap_or("");
        assert!(
            !quit_section.contains(r#"WritePropertyName(\"c\")"#),
            "STJ adjacent Write unit variant should NOT write content key:\n{quit_section}"
        );
    }

    #[test]
    fn adjacent_stj_write_data_has_content() {
        let config = stj_config();
        let ir = sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = newtonsoft_config();
        let ir = sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = newtonsoft_config();
        let ir = sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = newtonsoft_config();
        let ir = sample_adjacent_tagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // The unit variant Write arm should have WritePropertyName for tag
        // and WriteValue for the variant name, but no content key.
        assert!(
            tokens.contains("case Quit"),
            "Newtonsoft adjacent Write should have case Quit:\n{tokens}"
        );

        // Extract the Quit case block to verify no content key.
        let quit_section = tokens
            .split("case Quit")
            .nth(1)
            .and_then(|s| s.split("break;").next())
            .unwrap_or("");
        assert!(
            !quit_section.contains(r#"WritePropertyName(\"c\")"#),
            "Newtonsoft adjacent Write unit variant should NOT write content key:\n{quit_section}"
        );
    }

    // --- Untagged enum test helpers ---

    /// Builds an untagged enum IR with a struct variant (`Request`), a newtype
    /// variant (`Text`), and a unit variant (`Quit`).
    fn sample_untagged_enum_ir() -> DerivedCSharp {
        DerivedCSharp {
            rust_ident: quote::format_ident!("Message"),
            csharp_name: String::from("Message"),
            namespace: CSharpNamespace::new("Test.Ns").unwrap(),
            kind: DerivedCSharpKind::TaggedEnum {
                tagging: EnumTagging::Untagged,
                variants: vec![
                    TaggedVariant {
                        csharp_name: String::from("Request"),
                        json_name: String::from("Request"),
                        data: TaggedVariantData::Struct(vec![CSharpField {
                            csharp_property_name: String::from("Id"),
                            json_name: String::from("id"),
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
                            is_optional: false,
                        }]),
                    },
                    TaggedVariant {
                        csharp_name: String::from("Text"),
                        json_name: String::from("Text"),
                        data: TaggedVariantData::Newtype {
                            type_expr: quote! { <String as csharp_rs::CSharp>::csharp_name() },
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
        }
    }

    // --- Untagged converter tests ---

    #[test]
    fn untagged_stj_has_converter() {
        let config = stj_config();
        let ir = sample_untagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_config();
        let ir = sample_untagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = stj_config();
        let ir = sample_untagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // Unit variant should check for JsonValueKind.Null.
        assert!(
            tokens.contains("ValueKind.Null"),
            "untagged STJ Read should check ValueKind.Null for unit variants:\n{tokens}"
        );
        assert!(
            tokens.contains("new Quit()"),
            "untagged STJ Read should construct Quit with empty ctor:\n{tokens}"
        );
    }

    #[test]
    fn untagged_stj_write_unit_writes_null() {
        let config = stj_config();
        let ir = sample_untagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // Unit variant Write should emit null.
        assert!(
            tokens.contains("WriteNullValue"),
            "untagged STJ Write should use WriteNullValue for unit variants:\n{tokens}"
        );
    }

    #[test]
    fn untagged_stj_write_newtype_direct() {
        let config = stj_config();
        let ir = sample_untagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = newtonsoft_config();
        let ir = sample_untagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = newtonsoft_config();
        let ir = sample_untagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

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
        let config = newtonsoft_config();
        let ir = sample_untagged_enum_ir();
        let tokens = ir.into_token_stream(&config).to_string();

        // Unit variant Write should emit null via WriteNull().
        assert!(
            tokens.contains("WriteNull()"),
            "untagged Newtonsoft Write should use WriteNull() for unit variants:\n{tokens}"
        );
    }
}
