// Rust guideline compliant 2026-02-10
//! Named struct processing for C# code generation.
//!
//! Converts a Rust struct with named fields into the [`DerivedCSharp`]
//! intermediate representation, applying serde `rename_all` conventions
//! and mapping Rust types to C# equivalents.

use crate::attr::container::ContainerAttr;
use crate::attr::field::FieldAttr;
use crate::attr::to_pascal_case;
use crate::config::{CSharpConfig, CSharpNamespace};
use crate::types::{CSharpField, DerivedCSharp};
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
    config: &CSharpConfig,
) -> syn::Result<DerivedCSharp> {
    let rust_ident = input.ident.clone();
    // Use the Rust ident directly; struct names are already PascalCase by convention.
    let csharp_name = rust_ident.to_string();

    let namespace = match &container.namespace {
        Some(ns) => CSharpNamespace::new(ns.as_str()).map_err(|msg| {
            syn::Error::new_spanned(&input.ident, format!("csharp-rs: invalid namespace: {msg}"))
        })?,
        None => config.namespace.clone(),
    };

    let mut fields = Vec::new();

    for field in &named.named {
        let _field_attr = FieldAttr::from_attrs(&field.attrs)?;

        let field_ident = field
            .ident
            .as_ref()
            .expect("named fields always have identifiers");
        let field_name = field_ident.to_string();

        // JSON name: apply serde rename_all if present, otherwise use raw field name
        let json_name = container.rename_all.map_or_else(
            || field_name.clone(),
            |inflection| inflection.apply(&field_name),
        );

        // C# property name: always PascalCase
        let csharp_property_name = to_pascal_case(&field_name);

        // Determine if the type is Option<T> and extract the inner type
        let (is_optional, type_expr) = analyze_type(&field.ty);

        fields.push(CSharpField {
            csharp_property_name,
            json_name,
            type_expr,
            is_optional,
        });
    }

    Ok(DerivedCSharp {
        rust_ident,
        csharp_name,
        namespace,
        fields,
        export: container.export,
        export_to: container.export_to.clone(),
    })
}

/// Analyzes a Rust type to determine optionality and generate a type expression.
///
/// Returns `(is_optional, type_expr)` where `type_expr` is a `TokenStream`
/// that calls `<T as CSharp>::csharp_name()` at compile time.
fn analyze_type(ty: &Type) -> (bool, TokenStream) {
    if let Some(inner) = extract_option_inner(ty) {
        let expr = type_to_token_expr(inner);
        (true, expr)
    } else {
        let expr = type_to_token_expr(ty);
        (false, expr)
    }
}

/// Generates a token expression that resolves to the C# type name.
fn type_to_token_expr(ty: &Type) -> TokenStream {
    quote! { <#ty as csharp_rs::CSharp>::csharp_name() }
}

/// Extracts the inner type `T` from `Option<T>`, if the type is `Option`.
///
/// Only matches the bare identifier `Option`; fully-qualified paths like
/// `std::option::Option<T>` are not recognized. This matches serde's behavior.
fn extract_option_inner(ty: &Type) -> Option<&Type> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn extract_option_returns_inner_type() {
        let ty: Type = parse_quote!(Option<i32>);
        let inner = extract_option_inner(&ty);
        assert!(inner.is_some(), "should extract inner type from Option<i32>");
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
}
