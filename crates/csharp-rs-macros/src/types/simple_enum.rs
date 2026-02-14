// Rust guideline compliant 2026-02-10
//! Simple enum processing for C# code generation.
//!
//! Converts a Rust enum with only unit variants into the [`DerivedCSharp`]
//! intermediate representation, applying serde `rename_all` conventions
//! and mapping variants to C# enum members.

use crate::attr::container::ContainerAttr;
use crate::attr::field::FieldAttr;
use crate::attr::from_pascal_to_snake;
use crate::config::{CSharpConfig, CSharpNamespace};
use crate::types::{CSharpVariant, DerivedCSharp, DerivedCSharpKind};
use syn::{DataEnum, DeriveInput, Fields};

/// Builds a [`DerivedCSharp`] from an enum with only unit variants.
///
/// # Errors
///
/// Returns a `syn::Error` if any variant has fields (tuple or struct),
/// or if variant attributes are invalid.
pub fn simple_enum(
    input: &DeriveInput,
    enum_data: &DataEnum,
    container: &ContainerAttr,
    config: &CSharpConfig,
) -> syn::Result<DerivedCSharp> {
    let rust_ident = input.ident.clone();
    let csharp_name = rust_ident.to_string();

    let namespace = match &container.namespace {
        Some(ns) => CSharpNamespace::new(ns.as_str())
            .expect("namespace was validated in ContainerAttr::parse_csharp"),
        None => config.namespace.clone(),
    };

    let mut variants = Vec::new();

    for variant in &enum_data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                "csharp-rs: only unit variants are supported in enums",
            ));
        }

        let field_attr = FieldAttr::from_attrs(&variant.attrs)?;

        if field_attr.skip {
            continue;
        }

        let variant_name = variant.ident.to_string();

        // JSON name: variant rename overrides container rename_all.
        // Inflection expects snake_case input, so convert PascalCase first.
        let json_name = if let Some(ref renamed) = field_attr.rename {
            renamed.clone()
        } else {
            container.rename_all.map_or_else(
                || variant_name.clone(),
                |inflection| inflection.apply(&from_pascal_to_snake(&variant_name)),
            )
        };

        variants.push(CSharpVariant {
            csharp_name: variant_name,
            json_name,
        });
    }

    Ok(DerivedCSharp {
        rust_ident,
        csharp_name,
        namespace,
        kind: DerivedCSharpKind::Enum(variants),
        export: container.export,
        export_to: container.export_to.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn default_config() -> CSharpConfig {
        CSharpConfig::default()
    }

    fn process_enum(input: &DeriveInput) -> DerivedCSharp {
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Enum(ref enum_data) = input.data else {
            panic!("expected enum");
        };
        simple_enum(input, enum_data, &container, &default_config()).unwrap()
    }

    fn extract_variants(ir: &DerivedCSharp) -> &[CSharpVariant] {
        match &ir.kind {
            DerivedCSharpKind::Enum(variants) => variants,
            _ => panic!("expected Enum kind"),
        }
    }

    #[test]
    fn basic_unit_variants() {
        let input: DeriveInput = parse_quote! {
            enum Color {
                Red,
                Green,
                Blue,
            }
        };
        let ir = process_enum(&input);
        assert_eq!(ir.csharp_name, "Color");
        let variants = extract_variants(&ir);
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].csharp_name, "Red");
        assert_eq!(variants[0].json_name, "Red");
        assert_eq!(variants[1].csharp_name, "Green");
        assert_eq!(variants[2].csharp_name, "Blue");
    }

    #[test]
    fn rename_all_camel_case() {
        let input: DeriveInput = parse_quote! {
            #[serde(rename_all = "camelCase")]
            enum Status {
                InProgress,
                AlreadyDone,
                NotStarted,
            }
        };
        let ir = process_enum(&input);
        let variants = extract_variants(&ir);
        assert_eq!(variants[0].json_name, "inProgress");
        assert_eq!(variants[0].csharp_name, "InProgress");
        assert_eq!(variants[1].json_name, "alreadyDone");
        assert_eq!(variants[2].json_name, "notStarted");
    }

    #[test]
    fn per_variant_rename() {
        let input: DeriveInput = parse_quote! {
            enum Direction {
                #[serde(rename = "up")]
                North,
                South,
            }
        };
        let ir = process_enum(&input);
        let variants = extract_variants(&ir);
        assert_eq!(variants[0].json_name, "up");
        assert_eq!(variants[0].csharp_name, "North");
        assert_eq!(variants[1].json_name, "South");
    }

    #[test]
    fn skip_variant() {
        let input: DeriveInput = parse_quote! {
            enum Fruit {
                Apple,
                #[serde(skip)]
                Internal,
                Banana,
            }
        };
        let ir = process_enum(&input);
        let variants = extract_variants(&ir);
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].csharp_name, "Apple");
        assert_eq!(variants[1].csharp_name, "Banana");
    }

    #[test]
    fn reject_tuple_variant() {
        let input: DeriveInput = parse_quote! {
            enum Message {
                Quit,
                Data(String),
            }
        };
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Enum(ref enum_data) = input.data else {
            panic!("expected enum");
        };
        let result = simple_enum(&input, enum_data, &container, &default_config());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("only unit variants"),
            "error should mention unit variants: {err}"
        );
    }

    #[test]
    fn reject_struct_variant() {
        let input: DeriveInput = parse_quote! {
            enum Event {
                Click { x: i32, y: i32 },
            }
        };
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Enum(ref enum_data) = input.data else {
            panic!("expected enum");
        };
        let result = simple_enum(&input, enum_data, &container, &default_config());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("only unit variants"),
            "error should mention unit variants: {err}"
        );
    }
}
