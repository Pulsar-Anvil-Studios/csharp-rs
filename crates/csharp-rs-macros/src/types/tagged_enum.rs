// Rust guideline compliant 2026-03-14
//! Tagged enum processing for C# code generation.
//!
//! Converts a Rust enum with data variants into the [`DerivedCSharp`]
//! intermediate representation for tagged C# type hierarchies.

use crate::attr::container::ContainerAttr;
use crate::attr::field::FieldAttr;
use crate::attr::from_pascal_to_snake;
use crate::attr::to_pascal_case;
use crate::types::named::{analyze_type, extract_hashmap_types, type_to_token_expr};
use crate::types::{
    CSharpField, DerivedCSharp, DerivedCSharpKind, EnumTagging, FlattenKind, TaggedVariant,
    TaggedVariantData,
};
use syn::{DataEnum, DeriveInput, Fields};

/// Resolves the [`EnumTagging`] strategy from container attributes.
///
/// Follows serde's rules for `tag`, `content`, and `untagged` attributes.
///
/// # Errors
///
/// Returns a `syn::Error` if `content` is specified without `tag`.
fn resolve_tagging(container: &ContainerAttr, span: proc_macro2::Span) -> syn::Result<EnumTagging> {
    match (&container.tag, &container.content, container.untagged) {
        (_, _, true) => Ok(EnumTagging::Untagged),
        (Some(tag), Some(content), _) => Ok(EnumTagging::Adjacent {
            tag: tag.clone(),
            content: content.clone(),
        }),
        (Some(tag), None, _) => Ok(EnumTagging::Internal { tag: tag.clone() }),
        (None, None, false) => Ok(EnumTagging::External),
        (None, Some(_), false) => Err(syn::Error::new(
            span,
            "csharp-rs: serde(content) requires serde(tag)",
        )),
    }
}

/// Processes the named fields of a struct variant into [`CSharpField`] entries.
///
/// Applies the same field-level attribute handling as [`named_struct`](super::named::named_struct):
/// `rename`, `skip`, `skip_serializing`, `skip_serializing_if`.
fn process_struct_fields(
    fields: &syn::FieldsNamed,
    container: &ContainerAttr,
) -> syn::Result<Vec<CSharpField>> {
    let mut result = Vec::new();

    for field in &fields.named {
        let field_attr = FieldAttr::from_attrs(&field.attrs)?;

        if field_attr.skip {
            continue;
        }

        // Flatten fields: inline struct properties or emit extension data.
        if field_attr.flatten {
            if let Some((key_ty, value_ty)) = extract_hashmap_types(&field.ty) {
                let key_expr = type_to_token_expr(key_ty);
                let value_expr = type_to_token_expr(value_ty);
                result.push(CSharpField {
                    csharp_property_name: String::new(),
                    json_name: String::new(),
                    type_expr: proc_macro2::TokenStream::new(),
                    is_optional: false,
                    flatten: FlattenKind::HashMap {
                        key_expr,
                        value_expr,
                    },
                });
            } else {
                let ty = &field.ty;
                result.push(CSharpField {
                    csharp_property_name: String::new(),
                    json_name: String::new(),
                    type_expr: quote::quote! { <#ty as csharp_rs::CSharp>::csharp_fields(cfg) },
                    is_optional: false,
                    flatten: FlattenKind::Struct,
                });
            }
            continue;
        }

        let field_ident = field
            .ident
            .as_ref()
            .expect("named fields always have identifiers");
        let field_name = field_ident.to_string();

        // JSON name: field rename > rename_all_fields > rename_all > original
        let json_name = if let Some(ref renamed) = field_attr.rename {
            renamed.clone()
        } else {
            let inflection = container.rename_all_fields.or(container.rename_all);
            inflection.map_or_else(
                || field_name.clone(),
                |inf| inf.apply(&field_name),
            )
        };

        // C# property name: always PascalCase.
        let csharp_property_name = to_pascal_case(&field_name);

        // Type analysis -- skip_serializing_if and default both force nullable.
        let (is_optional, type_expr) = analyze_type(&field.ty);
        let is_optional = is_optional || field_attr.skip_serializing_if || field_attr.default;

        // Apply explicit C# type override if specified via #[csharp(type = "...")].
        let type_expr = if let Some(ref override_type) = field_attr.type_override {
            quote::quote! { String::from(#override_type) }
        } else {
            type_expr
        };

        result.push(CSharpField {
            csharp_property_name,
            json_name,
            type_expr,
            is_optional,
            flatten: FlattenKind::None,
        });
    }

    Ok(result)
}

/// Builds a [`DerivedCSharp`] from an enum with data variants.
///
/// Handles all four serde tagging strategies: external, internal,
/// adjacent, and untagged. Supports unit, newtype, and struct variants.
///
/// # Errors
///
/// Returns a `syn::Error` if:
/// - `content` is specified without `tag`
/// - A tuple variant has multiple fields
/// - Variant or field attributes are invalid
pub fn tagged_enum(
    input: &DeriveInput,
    enum_data: &DataEnum,
    container: &ContainerAttr,
) -> syn::Result<DerivedCSharp> {
    let rust_ident = input.ident.clone();
    // Use the Rust ident directly; enum names are already PascalCase by convention.
    let csharp_name = rust_ident.to_string();

    let namespace_override = container.namespace.clone();

    let tagging = resolve_tagging(container, input.ident.span())?;

    let mut variants = Vec::new();

    for variant in &enum_data.variants {
        let field_attr = FieldAttr::from_attrs(&variant.attrs)?;

        if field_attr.skip {
            continue;
        }

        let variant_name = variant.ident.to_string();

        // JSON name: per-variant rename overrides container rename_all.
        // Inflection expects snake_case input, so convert PascalCase first.
        let json_name = if let Some(ref renamed) = field_attr.rename {
            renamed.clone()
        } else {
            container.rename_all.map_or_else(
                || variant_name.clone(),
                |inflection| inflection.apply(&from_pascal_to_snake(&variant_name)),
            )
        };

        // C# name: variant ident as-is (already PascalCase).
        let csharp_name = variant_name;

        // Determine variant data from field shape.
        let data = match &variant.fields {
            Fields::Unit => TaggedVariantData::Unit,
            Fields::Unnamed(unnamed) => {
                if unnamed.unnamed.len() == 1 {
                    let ty = &unnamed.unnamed[0].ty;
                    let type_expr = type_to_token_expr(ty);
                    TaggedVariantData::Newtype { type_expr }
                } else {
                    return Err(syn::Error::new_spanned(
                        &variant.ident,
                        "csharp-rs: tuple variants with multiple fields are not supported",
                    ));
                }
            }
            Fields::Named(named) => {
                let fields = process_struct_fields(named, container)?;
                TaggedVariantData::Struct(fields)
            }
        };

        variants.push(TaggedVariant {
            csharp_name,
            json_name,
            data,
        });
    }

    Ok(DerivedCSharp {
        rust_ident,
        csharp_name,
        namespace_override,
        kind: DerivedCSharpKind::TaggedEnum { tagging, variants },
        export: container.export,
        export_to: container.export_to.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn process(input: &DeriveInput) -> DerivedCSharp {
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Enum(ref enum_data) = input.data else {
            panic!("expected enum")
        };
        tagged_enum(input, enum_data, &container).unwrap()
    }

    fn extract_variants(ir: &DerivedCSharp) -> (&EnumTagging, &[TaggedVariant]) {
        match &ir.kind {
            DerivedCSharpKind::TaggedEnum { tagging, variants } => (tagging, variants),
            _ => panic!("expected TaggedEnum kind"),
        }
    }

    #[test]
    fn internal_tagging_parsed() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Message {
                Request { id: String },
                Quit,
            }
        };
        let ir = process(&input);
        let (tagging, variants) = extract_variants(&ir);
        assert!(matches!(tagging, EnumTagging::Internal { tag } if tag == "type"));
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn adjacent_tagging_parsed() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "t", content = "c")]
            enum Block {
                Para(String),
                Code { lang: String, body: String },
            }
        };
        let ir = process(&input);
        let (tagging, variants) = extract_variants(&ir);
        assert!(
            matches!(tagging, EnumTagging::Adjacent { tag, content } if tag == "t" && content == "c")
        );
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn external_tagging_default() {
        let input: DeriveInput = parse_quote! {
            enum Message {
                Text(String),
                Quit,
            }
        };
        let ir = process(&input);
        let (tagging, _) = extract_variants(&ir);
        assert!(matches!(tagging, EnumTagging::External));
    }

    #[test]
    fn untagged_parsed() {
        let input: DeriveInput = parse_quote! {
            #[serde(untagged)]
            enum Data {
                Text(String),
                Number { value: f64 },
            }
        };
        let ir = process(&input);
        let (tagging, _) = extract_variants(&ir);
        assert!(matches!(tagging, EnumTagging::Untagged));
    }

    #[test]
    fn struct_variant_produces_struct_data() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Event {
                Click { x: i32, y: i32 },
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert_eq!(variants[0].csharp_name, "Click");
        assert!(
            matches!(&variants[0].data, TaggedVariantData::Struct(fields) if fields.len() == 2)
        );
    }

    #[test]
    fn newtype_variant_produces_newtype_data() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "t", content = "c")]
            enum Value {
                Text(String),
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert!(matches!(
            &variants[0].data,
            TaggedVariantData::Newtype { .. }
        ));
    }

    #[test]
    fn unit_variant_produces_unit_data() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Message {
                Quit,
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert!(matches!(&variants[0].data, TaggedVariantData::Unit));
    }

    #[test]
    fn tuple_variant_rejected() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Msg {
                Data(String, i32),
            }
        };
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Enum(ref enum_data) = input.data else {
            panic!("expected enum")
        };
        let result = tagged_enum(&input, enum_data, &container);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("tuple variants"));
    }

    #[test]
    fn content_without_tag_errors() {
        let input: DeriveInput = parse_quote! {
            #[serde(content = "c")]
            enum Msg {
                Text(String),
            }
        };
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Enum(ref enum_data) = input.data else {
            panic!("expected enum")
        };
        let result = tagged_enum(&input, enum_data, &container);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("content"));
    }

    #[test]
    fn rename_all_applies_to_variant_json_names() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type", rename_all = "camelCase")]
            enum Event {
                UserLogin { user_id: String },
                SessionEnd,
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert_eq!(variants[0].json_name, "userLogin");
        assert_eq!(variants[0].csharp_name, "UserLogin");
        assert_eq!(variants[1].json_name, "sessionEnd");
    }

    #[test]
    fn per_variant_rename_overrides_rename_all() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type", rename_all = "camelCase")]
            enum Event {
                #[serde(rename = "CLICK")]
                Click { x: i32 },
                Move { dx: i32 },
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert_eq!(variants[0].json_name, "CLICK");
        assert_eq!(variants[1].json_name, "move");
    }

    #[test]
    fn skip_variant_excluded() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Message {
                Text(String),
                #[serde(skip)]
                Internal,
                Quit,
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].csharp_name, "Text");
        assert_eq!(variants[1].csharp_name, "Quit");
    }

    #[test]
    fn struct_variant_fields_respect_field_attrs() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Event {
                Click {
                    #[serde(rename = "posX")]
                    x: i32,
                    #[serde(skip)]
                    internal: String,
                    y: i32,
                },
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        match &variants[0].data {
            TaggedVariantData::Struct(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].json_name, "posX");
                assert_eq!(fields[0].csharp_property_name, "X");
                assert_eq!(fields[1].csharp_property_name, "Y");
            }
            _ => panic!("expected Struct data"),
        }
    }

    #[test]
    fn struct_variant_flatten_field() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Event {
                Action {
                    name: String,
                    #[serde(flatten)]
                    meta: Metadata,
                },
            }
        };
        let ir = tagged_enum(
            &input,
            match &input.data {
                syn::Data::Enum(e) => e,
                _ => panic!("expected enum"),
            },
            &ContainerAttr::from_attrs(&input.attrs).unwrap(),
        )
        .unwrap();

        match &ir.kind {
            DerivedCSharpKind::TaggedEnum { variants, .. } => match &variants[0].data {
                TaggedVariantData::Struct(fields) => {
                    assert_eq!(fields.len(), 2);
                    assert!(matches!(fields[0].flatten, FlattenKind::None));
                    assert!(matches!(fields[1].flatten, FlattenKind::Struct));
                }
                _ => panic!("expected Struct variant data"),
            },
            _ => panic!("expected TaggedEnum kind"),
        }
    }

    #[test]
    fn rename_all_fields_overrides_rename_all_for_struct_variant_fields() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type", rename_all = "UPPERCASE", rename_all_fields = "camelCase")]
            enum Event {
                UserLogin { user_name: String, login_time: String },
                SessionEnd,
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);

        // Variant discriminators should use rename_all (UPPERCASE).
        assert_eq!(variants[0].json_name, "USERLOGIN");
        assert_eq!(variants[1].json_name, "SESSIONEND");

        // Struct variant fields should use rename_all_fields (camelCase).
        match &variants[0].data {
            TaggedVariantData::Struct(fields) => {
                assert_eq!(
                    fields[0].json_name, "userName",
                    "rename_all_fields should apply to struct variant fields"
                );
                assert_eq!(fields[1].json_name, "loginTime");
            }
            _ => panic!("expected Struct data"),
        }
    }

    #[test]
    fn field_rename_overrides_rename_all_fields_in_variant() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type", rename_all_fields = "camelCase")]
            enum Event {
                Click {
                    #[serde(rename = "posX")]
                    x_pos: i32,
                    y_pos: i32,
                },
            }
        };
        let ir = process(&input);
        let (_, variants) = extract_variants(&ir);
        match &variants[0].data {
            TaggedVariantData::Struct(fields) => {
                assert_eq!(
                    fields[0].json_name, "posX",
                    "per-field rename should override rename_all_fields"
                );
                assert_eq!(
                    fields[1].json_name, "yPos",
                    "rename_all_fields should apply when no per-field rename"
                );
            }
            _ => panic!("expected Struct data"),
        }
    }
}
