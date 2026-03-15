// Rust guideline compliant 2026-03-15
//! Generic type parameter analysis utilities.
//!
//! Provides helpers for determining which type parameters from a struct or
//! enum declaration are actually used in field types, and for generating
//! the appropriate `where` clause bounds for the `CSharp` trait impl.

use std::collections::{HashMap, HashSet};

use proc_macro2::Ident;
use syn::punctuated::Punctuated;
use syn::{
    AngleBracketedGenericArguments, GenericArgument, Generics, PathArguments, QSelf, Token, Type,
    TypeArray, TypeParen, TypePath, TypeReference, TypeSlice, TypeTuple, WherePredicate,
};

/// Recursively extracts type parameters used within a given type.
///
/// Associated types of a type parameter (e.g., `I::Item`) are extracted
/// as the full path, not just the bare parameter.
///
/// Ported from ts-rs `used_type_params`.
pub fn used_type_params<'ty>(
    out: &mut HashSet<&'ty Type>,
    ty: &'ty Type,
    is_type_param: &dyn Fn(&Ident) -> bool,
) {
    match ty {
        Type::Array(TypeArray { elem, .. })
        | Type::Paren(TypeParen { elem, .. })
        | Type::Reference(TypeReference { elem, .. })
        | Type::Slice(TypeSlice { elem, .. }) => used_type_params(out, elem, is_type_param),

        Type::Tuple(TypeTuple { elems, .. }) => {
            for elem in elems {
                used_type_params(out, elem, is_type_param);
            }
        }

        Type::Path(TypePath { qself: None, path }) => {
            let Some(first) = path.segments.first() else {
                return;
            };
            if is_type_param(&first.ident) {
                // Either a bare type param (`T`) or an associated type (`T::Item`).
                out.insert(ty);
                return;
            }

            // Recurse into angle-bracketed generic arguments (e.g., `Vec<T>`).
            let Some(last) = path.segments.last() else {
                return;
            };
            if let PathArguments::AngleBracketed(AngleBracketedGenericArguments {
                ref args, ..
            }) = last.arguments
            {
                for generic in args {
                    if let GenericArgument::Type(ty) = generic {
                        used_type_params(out, ty, is_type_param);
                    }
                }
            }
        }

        Type::Path(TypePath {
            qself: Some(QSelf { ty, .. }),
            ..
        }) => {
            used_type_params(out, ty, is_type_param);
        }

        _ => {}
    }
}

/// Collects all type parameters from `generics` that appear in `field_types`.
///
/// Returns the set of used type references (which may include associated
/// types like `T::Item`).
pub fn collect_used_params<'ty>(
    generics: &Generics,
    field_types: impl Iterator<Item = &'ty Type>,
) -> HashSet<&'ty Type> {
    let is_type_param = |id: &Ident| generics.type_params().any(|p| &p.ident == id);
    let mut used = HashSet::new();
    for ty in field_types {
        used_type_params(&mut used, ty, &is_type_param);
    }
    used
}

/// Generates the where-clause predicates for a `CSharp` trait impl.
///
/// If `custom_bounds` is `Some`, those predicates are used directly
/// (from `#[csharp(bound = "...")]`). Otherwise, auto-generates
/// `T: csharp_rs::CSharp` bounds for each type parameter actually used
/// in field types, excluding params listed in `concrete`.
///
/// Always preserves any existing where-clause predicates from the source
/// type declaration.
pub fn build_where_predicates(
    generics: &Generics,
    field_types: &[&Type],
    concrete: &HashMap<Ident, syn::Type>,
    custom_bounds: Option<&Vec<WherePredicate>>,
) -> Punctuated<WherePredicate, Token![,]> {
    let mut predicates = Punctuated::new();

    // Preserve existing where-clause predicates from the Rust type.
    if let Some(where_clause) = &generics.where_clause {
        for pred in &where_clause.predicates {
            predicates.push(pred.clone());
        }
    }

    if let Some(bounds) = custom_bounds {
        // User-specified bounds override auto-detection.
        for pred in bounds {
            predicates.push(pred.clone());
        }
    } else {
        // Auto-generate `T: csharp_rs::CSharp` for each used, non-concrete param.
        let used = collect_used_params(generics, field_types.iter().copied());
        for ty in used {
            // Skip params that are in the concrete map.
            if let Type::Path(TypePath { qself: None, path }) = ty {
                if let Some(first) = path.segments.first() {
                    if concrete.contains_key(&first.ident) {
                        continue;
                    }
                }
            }
            let pred: WherePredicate = syn::parse_quote!(#ty: csharp_rs::CSharp);
            predicates.push(pred);
        }
    }

    predicates
}

/// Returns `true` if the generics contain any type parameters.
pub fn has_type_params(generics: &Generics) -> bool {
    generics.type_params().next().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn bare_type_param_is_found() {
        let generics: Generics = parse_quote!(<T>);
        let ty: Type = parse_quote!(T);
        let used = collect_used_params(&generics, std::iter::once(&ty));
        assert_eq!(used.len(), 1, "should find bare T");
    }

    #[test]
    fn nested_type_param_in_vec() {
        let generics: Generics = parse_quote!(<T>);
        let ty: Type = parse_quote!(Vec<T>);
        let used = collect_used_params(&generics, std::iter::once(&ty));
        assert_eq!(used.len(), 1, "should find T inside Vec<T>");
    }

    #[test]
    fn multiple_params_in_hashmap() {
        let generics: Generics = parse_quote!(<K, V>);
        let ty: Type = parse_quote!(HashMap<K, V>);
        let used = collect_used_params(&generics, std::iter::once(&ty));
        assert_eq!(used.len(), 2, "should find both K and V");
    }

    #[test]
    fn non_param_type_is_ignored() {
        let generics: Generics = parse_quote!(<T>);
        let ty: Type = parse_quote!(String);
        let used = collect_used_params(&generics, std::iter::once(&ty));
        assert!(used.is_empty(), "String is not a type param");
    }

    #[test]
    fn option_wrapping_param() {
        let generics: Generics = parse_quote!(<T>);
        let ty: Type = parse_quote!(Option<T>);
        let used = collect_used_params(&generics, std::iter::once(&ty));
        assert_eq!(used.len(), 1, "should find T inside Option<T>");
    }

    #[test]
    fn deeply_nested_param() {
        let generics: Generics = parse_quote!(<T>);
        let ty: Type = parse_quote!(Vec<Option<T>>);
        let used = collect_used_params(&generics, std::iter::once(&ty));
        assert_eq!(used.len(), 1, "should find T in Vec<Option<T>>");
    }

    #[test]
    fn tuple_with_params() {
        let generics: Generics = parse_quote!(<A, B>);
        let ty: Type = parse_quote!((A, B));
        let used = collect_used_params(&generics, std::iter::once(&ty));
        assert_eq!(used.len(), 2, "should find both params in tuple");
    }

    #[test]
    fn reference_to_param() {
        let generics: Generics = parse_quote!(<T>);
        let ty: Type = parse_quote!(&T);
        let used = collect_used_params(&generics, std::iter::once(&ty));
        assert_eq!(used.len(), 1, "should find T through reference");
    }

    #[test]
    fn unused_param_not_collected() {
        let generics: Generics = parse_quote!(<T, U>);
        let ty: Type = parse_quote!(T);
        let used = collect_used_params(&generics, std::iter::once(&ty));
        assert_eq!(used.len(), 1, "only T should be found, not U");
    }

    #[test]
    fn has_type_params_true() {
        let generics: Generics = parse_quote!(<T>);
        assert!(has_type_params(&generics));
    }

    #[test]
    fn has_type_params_false_empty() {
        let generics: Generics = Generics::default();
        assert!(!has_type_params(&generics));
    }

    #[test]
    fn has_type_params_false_lifetime_only() {
        let generics: Generics = parse_quote!(<'a>);
        assert!(!has_type_params(&generics));
    }

    #[test]
    fn build_where_skips_concrete_params() {
        let generics: Generics = parse_quote!(<T, U>);
        let ty_t: Type = parse_quote!(T);
        let ty_u: Type = parse_quote!(U);
        let field_types: Vec<&Type> = vec![&ty_t, &ty_u];
        let mut concrete = HashMap::new();
        concrete.insert(parse_quote!(T), parse_quote!(String));

        let preds = build_where_predicates(&generics, &field_types, &concrete, None);

        // Only U should get a bound, not T (which is concrete).
        assert_eq!(preds.len(), 1, "should only have bound for U");
        let pred_str = preds.first().map(|p| quote::quote!(#p).to_string());
        assert!(
            pred_str
                .as_ref()
                .is_some_and(|s| s.contains('U') && !s.contains('T')),
            "bound should be for U: {pred_str:?}"
        );
    }

    #[test]
    fn build_where_uses_custom_bounds() {
        let generics: Generics = parse_quote!(<T>);
        let ty: Type = parse_quote!(T);
        let field_types: Vec<&Type> = vec![&ty];
        let custom: Vec<WherePredicate> = vec![parse_quote!(T: std::fmt::Display)];

        let preds = build_where_predicates(&generics, &field_types, &HashMap::new(), Some(&custom));
        assert_eq!(preds.len(), 1);
        let pred_str = preds.first().map(|p| quote::quote!(#p).to_string());
        assert!(
            pred_str.as_ref().is_some_and(|s| s.contains("Display")),
            "should use custom bound: {pred_str:?}"
        );
    }

    #[test]
    fn build_where_preserves_existing_clause() {
        // Parse a full item to get Generics with a where clause.
        let item: syn::ItemStruct = parse_quote! {
            struct Foo<T> where T: Clone { value: T }
        };
        let ty: Type = parse_quote!(T);
        let field_types: Vec<&Type> = vec![&ty];

        let preds = build_where_predicates(&item.generics, &field_types, &HashMap::new(), None);

        // Should have both: the existing `T: Clone` and auto-generated `T: CSharp`.
        assert_eq!(preds.len(), 2, "existing + auto-generated");
    }
}
