// Rust guideline compliant 2026-03-14
//! Named struct processing for C# code generation.
//!
//! Converts a Rust struct with named fields into the [`DerivedCSharp`]
//! intermediate representation, applying serde `rename_all` conventions
//! and mapping Rust types to C# equivalents.

use crate::attr::container::ContainerAttr;
use crate::attr::field::FieldAttr;
use crate::attr::to_pascal_case;
use crate::types::{CSharpField, DerivedCSharp, DerivedCSharpKind, FlattenKind};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, FieldsNamed, GenericArgument, PathArguments, Type, TypePath};

/// Builds a [`DerivedCSharp`] from a struct with named fields.
///
/// # Errors
///
/// Returns a `syn::Error` if field attributes are invalid.
pub fn named_struct(
    input: &DeriveInput,
    named: &FieldsNamed,
    container: &ContainerAttr,
) -> syn::Result<DerivedCSharp> {
    let rust_ident = input.ident.clone();
    // Use the Rust ident directly; struct names are already PascalCase by convention.
    let csharp_name = rust_ident.to_string();

    // Namespace: store only the per-type override; runtime config provides the default.
    let namespace_override = container.namespace.clone();

    let mut fields = Vec::new();

    for field in &named.named {
        let field_attr = FieldAttr::from_attrs(&field.attrs)?;

        // Skip fields marked with serde(skip) or serde(skip_serializing)
        if field_attr.skip {
            continue;
        }

        // Flatten fields: inline struct properties or emit extension data.
        if field_attr.flatten {
            if let Some((key_ty, value_ty)) = extract_hashmap_types(&field.ty) {
                let key_expr = type_to_token_expr(key_ty);
                let value_expr = type_to_token_expr(value_ty);
                fields.push(CSharpField {
                    csharp_property_name: String::new(),
                    json_name: String::new(),
                    type_expr: TokenStream::new(),
                    is_optional: false,
                    flatten: FlattenKind::HashMap {
                        key_expr,
                        value_expr,
                    },
                });
            } else {
                let ty = &field.ty;
                fields.push(CSharpField {
                    csharp_property_name: String::new(),
                    json_name: String::new(),
                    type_expr: quote! { <#ty as csharp_rs::CSharp>::csharp_fields(cfg) },
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
            inflection.map_or_else(|| field_name.clone(), |inf| inf.apply(&field_name))
        };

        // C# property name: always PascalCase
        let csharp_property_name = to_pascal_case(&field_name);

        // Type analysis -- skip_serializing_if and default both force nullable
        let (is_optional, type_expr) = analyze_type(&field.ty);
        let is_optional = is_optional || field_attr.skip_serializing_if || field_attr.default;

        // Apply explicit C# type override if specified via #[csharp(type = "...")].
        let type_expr = if let Some(ref override_type) = field_attr.type_override {
            quote! { String::from(#override_type) }
        } else {
            type_expr
        };

        fields.push(CSharpField {
            csharp_property_name,
            json_name,
            type_expr,
            is_optional,
            flatten: FlattenKind::None,
        });
    }

    Ok(DerivedCSharp {
        rust_ident,
        csharp_name,
        namespace_override,
        kind: DerivedCSharpKind::Record(fields),
        export: container.export,
        export_to: container.export_to.clone(),
        transparent: container.transparent,
    })
}

/// Analyzes a Rust type to determine optionality and generate a type expression.
///
/// Returns `(is_optional, type_expr)` where `type_expr` is a `TokenStream`
/// that calls `<T as CSharp>::csharp_name()` at compile time.
pub(crate) fn analyze_type(ty: &Type) -> (bool, TokenStream) {
    if let Some(inner) = extract_option_inner(ty) {
        let expr = type_to_token_expr(inner);
        (true, expr)
    } else {
        let expr = type_to_token_expr(ty);
        (false, expr)
    }
}

/// Generates a token expression that resolves to the C# type name.
pub(crate) fn type_to_token_expr(ty: &Type) -> TokenStream {
    quote! { <#ty as csharp_rs::CSharp>::csharp_name(cfg) }
}

/// Extracts the inner type `T` from `Option<T>`, if the type is `Option`.
///
/// Only matches the bare identifier `Option`; fully-qualified paths like
/// `std::option::Option<T>` are not recognized. This matches serde's behavior.
pub(crate) fn extract_option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(TypePath { path, .. }) = ty else {
        return None;
    };

    let segment = path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }

    let PathArguments::AngleBracketed(ref args) = segment.arguments else {
        return None;
    };

    if args.args.len() != 1 {
        return None;
    }

    match &args.args[0] {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

/// Extracts the key and value types from `HashMap<K, V>`, if the type is `HashMap`.
///
/// Only matches the bare identifier `HashMap`; fully-qualified paths are not
/// recognized. Returns `None` for non-HashMap types.
pub(crate) fn extract_hashmap_types(ty: &Type) -> Option<(&Type, &Type)> {
    let Type::Path(TypePath { path, .. }) = ty else {
        return None;
    };

    let segment = path.segments.last()?;
    if segment.ident != "HashMap" {
        return None;
    }

    let PathArguments::AngleBracketed(ref args) = segment.arguments else {
        return None;
    };

    if args.args.len() != 2 {
        return None;
    }

    match (&args.args[0], &args.args[1]) {
        (GenericArgument::Type(key), GenericArgument::Type(value)) => Some((key, value)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn process_named(input: &DeriveInput) -> DerivedCSharp {
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Struct(syn::DataStruct {
            fields: syn::Fields::Named(ref named),
            ..
        }) = input.data
        else {
            panic!("expected named struct");
        };
        named_struct(input, named, &container).unwrap()
    }

    fn extract_fields(ir: &DerivedCSharp) -> &[CSharpField] {
        match &ir.kind {
            DerivedCSharpKind::Record(fields) => fields,
            _ => panic!("expected Record kind"),
        }
    }

    #[test]
    fn extract_option_returns_inner_type() {
        let ty: Type = parse_quote!(Option<i32>);
        let inner = extract_option_inner(&ty);
        assert!(
            inner.is_some(),
            "should extract inner type from Option<i32>"
        );
    }

    #[test]
    fn extract_option_returns_none_for_non_option() {
        let ty: Type = parse_quote!(String);
        assert!(extract_option_inner(&ty).is_none());
    }

    #[test]
    fn extract_option_returns_none_for_vec() {
        let ty: Type = parse_quote!(Vec<i32>);
        assert!(extract_option_inner(&ty).is_none());
    }

    #[test]
    fn extract_option_nested_option() {
        let ty: Type = parse_quote!(Option<Option<i32>>);
        let inner = extract_option_inner(&ty);
        assert!(inner.is_some(), "should extract outer Option layer");
    }

    #[test]
    fn analyze_type_marks_option_as_optional() {
        let ty: Type = parse_quote!(Option<String>);
        let (is_optional, _) = analyze_type(&ty);
        assert!(is_optional);
    }

    #[test]
    fn analyze_type_marks_plain_type_as_required() {
        let ty: Type = parse_quote!(i32);
        let (is_optional, _) = analyze_type(&ty);
        assert!(!is_optional);
    }

    #[test]
    fn extract_option_returns_none_for_reference() {
        let ty: Type = parse_quote!(&str);
        assert!(extract_option_inner(&ty).is_none());
    }

    #[test]
    fn extract_option_returns_none_for_bare_option() {
        let ty: Type = parse_quote!(Option);
        assert!(extract_option_inner(&ty).is_none());
    }

    #[test]
    fn extract_option_returns_none_for_multi_arg() {
        let ty: Type = parse_quote!(Option<A, B>);
        assert!(extract_option_inner(&ty).is_none());
    }

    #[test]
    fn extract_option_returns_none_for_lifetime_arg() {
        let ty: Type = parse_quote!(Option<'a>);
        assert!(extract_option_inner(&ty).is_none());
    }

    #[test]
    fn named_struct_with_namespace_override() {
        let input: DeriveInput = parse_quote! {
            #[csharp(namespace = "Custom.Namespace")]
            struct Foo {
                x: i32,
            }
        };
        let ir = process_named(&input);
        assert_eq!(ir.namespace_override.as_deref(), Some("Custom.Namespace"));
    }

    #[test]
    fn field_rename_overrides_json_name() {
        let input: DeriveInput = parse_quote! {
            struct Foo {
                #[serde(rename = "userId")]
                user_id: String,
                level: i32,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(fields[0].json_name, "userId");
        assert_eq!(fields[0].csharp_property_name, "UserId");
        assert_eq!(fields[1].json_name, "level");
    }

    #[test]
    fn field_rename_with_rename_all() {
        let input: DeriveInput = parse_quote! {
            #[serde(rename_all = "camelCase")]
            struct Foo {
                #[serde(rename = "ID")]
                player_id: String,
                display_name: String,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(fields[0].json_name, "ID");
        assert_eq!(fields[1].json_name, "displayName");
    }

    #[test]
    fn field_skip_excludes_field() {
        let input: DeriveInput = parse_quote! {
            struct Foo {
                visible: String,
                #[serde(skip)]
                hidden: String,
                also_visible: i32,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].csharp_property_name, "Visible");
        assert_eq!(fields[1].csharp_property_name, "AlsoVisible");
    }

    #[test]
    fn field_skip_serializing_excludes_field() {
        let input: DeriveInput = parse_quote! {
            struct Foo {
                visible: String,
                #[serde(skip_serializing)]
                write_only: String,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].csharp_property_name, "Visible");
    }

    #[test]
    fn field_skip_serializing_if_forces_nullable() {
        let input: DeriveInput = parse_quote! {
            struct Foo {
                name: String,
                #[serde(skip_serializing_if = "String::is_empty")]
                tag: String,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(fields.len(), 2);
        assert!(!fields[0].is_optional, "name should not be optional");
        assert!(fields[1].is_optional, "tag should be forced optional");
    }

    #[test]
    fn field_serde_default_forces_nullable() {
        let input: DeriveInput = parse_quote! {
            struct Foo {
                name: String,
                #[serde(default)]
                level: i32,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(fields.len(), 2);
        assert!(!fields[0].is_optional, "name should not be optional");
        assert!(
            fields[1].is_optional,
            "level with default should be optional"
        );
    }

    #[test]
    fn extract_hashmap_returns_key_value_types() {
        let ty: Type = parse_quote!(HashMap<String, i32>);
        let result = extract_hashmap_types(&ty);
        assert!(result.is_some(), "should extract HashMap key/value types");
    }

    #[test]
    fn extract_hashmap_returns_none_for_non_hashmap() {
        let ty: Type = parse_quote!(Vec<i32>);
        assert!(extract_hashmap_types(&ty).is_none());
    }

    #[test]
    fn flatten_struct_field_creates_flatten_kind() {
        let input: DeriveInput = parse_quote! {
            struct Outer {
                name: String,
                #[serde(flatten)]
                inner: Inner,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(fields.len(), 2);
        assert!(
            matches!(fields[0].flatten, FlattenKind::None),
            "regular field should be FlattenKind::None"
        );
        assert!(
            matches!(fields[1].flatten, FlattenKind::Struct),
            "flatten field should be FlattenKind::Struct"
        );
    }

    #[test]
    fn flatten_hashmap_field_creates_hashmap_kind() {
        let input: DeriveInput = parse_quote! {
            struct WithExtra {
                name: String,
                #[serde(flatten)]
                extra: HashMap<String, Value>,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(fields.len(), 2);
        assert!(
            matches!(fields[1].flatten, FlattenKind::HashMap { .. }),
            "flatten HashMap field should be FlattenKind::HashMap"
        );
    }

    #[test]
    fn flatten_and_skip_excludes_field() {
        let input: DeriveInput = parse_quote! {
            struct Foo {
                name: String,
                #[serde(flatten, skip)]
                hidden: Inner,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(fields.len(), 1, "skip should take priority over flatten");
    }

    #[test]
    fn rename_all_fields_overrides_rename_all_for_fields() {
        let input: DeriveInput = parse_quote! {
            #[serde(rename_all = "UPPERCASE", rename_all_fields = "camelCase")]
            struct Foo {
                player_name: String,
                high_score: i32,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(
            fields[0].json_name, "playerName",
            "rename_all_fields camelCase should override rename_all UPPERCASE for fields"
        );
        assert_eq!(fields[1].json_name, "highScore");
    }

    #[test]
    fn type_override_replaces_type_expr() {
        let input: DeriveInput = parse_quote! {
            struct Foo {
                #[csharp(type = "JsonElement")]
                data: String,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(fields.len(), 1);
        let tokens = fields[0].type_expr.to_string();
        assert!(
            tokens.contains("JsonElement"),
            "type_expr should contain override string:\n{tokens}"
        );
        assert!(
            !tokens.contains("csharp_rs"),
            "type_expr should NOT contain trait call:\n{tokens}"
        );
    }

    #[test]
    fn field_rename_overrides_rename_all_fields() {
        let input: DeriveInput = parse_quote! {
            #[serde(rename_all_fields = "camelCase")]
            struct Foo {
                #[serde(rename = "ID")]
                player_id: String,
                display_name: String,
            }
        };
        let ir = process_named(&input);
        let fields = extract_fields(&ir);
        assert_eq!(
            fields[0].json_name, "ID",
            "per-field rename should override rename_all_fields"
        );
        assert_eq!(
            fields[1].json_name, "displayName",
            "rename_all_fields should apply when no per-field rename"
        );
    }
}
