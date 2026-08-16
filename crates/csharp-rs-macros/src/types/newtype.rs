//! Newtype struct processing.
//!
//! Converts `struct Foo(Bar)` into a [`DerivedCSharp`] with a single
//! synthetic `Value` property, reusing the record codegen path.

use syn::DeriveInput;

use crate::attr::container::ContainerAttr;
use crate::types::named::analyze_type;
use crate::types::{CSharpField, DerivedCSharp, DerivedCSharpKind, FlattenKind};

/// Processes a newtype struct (single-field tuple struct) into a [`DerivedCSharp`].
///
/// The inner type becomes a `Value` property in the generated C# record.
#[expect(
    clippy::unnecessary_wraps,
    reason = "returns Result to match the dispatch signature in process_input"
)]
pub fn newtype_struct(
    input: &DeriveInput,
    fields: &syn::FieldsUnnamed,
    container: &ContainerAttr,
) -> syn::Result<DerivedCSharp> {
    let rust_ident = input.ident.clone();
    let csharp_name = rust_ident.to_string();
    let namespace_override = container.namespace.clone();

    let inner_field = fields.unnamed.first().expect("newtype has exactly 1 field");
    let (is_optional, type_expr) = analyze_type(&inner_field.ty);

    let field = CSharpField {
        csharp_property_name: String::from("Value"),
        json_name: String::from("Value"),
        type_expr,
        is_optional,
        flatten: FlattenKind::None,
    };

    Ok(DerivedCSharp {
        rust_ident,
        generics: input.generics.clone(),
        concrete: container.concrete.clone(),
        custom_bounds: container.bound.clone(),
        csharp_name,
        namespace_override,
        kind: DerivedCSharpKind::Record(vec![field]),
        export: container.export,
        export_to: container.export_to.clone(),
        transparent: container.transparent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn process_newtype(input: &DeriveInput) -> DerivedCSharp {
        let container = ContainerAttr::from_attrs(&input.attrs).unwrap();
        let syn::Data::Struct(syn::DataStruct {
            fields: syn::Fields::Unnamed(ref fields),
            ..
        }) = input.data
        else {
            panic!("expected unnamed struct");
        };
        newtype_struct(input, fields, &container).unwrap()
    }

    #[test]
    fn newtype_produces_value_property() {
        let input: DeriveInput = parse_quote! { struct UserId(String); };
        let ir = process_newtype(&input);
        assert_eq!(ir.csharp_name, "UserId");
        match &ir.kind {
            DerivedCSharpKind::Record(fields) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].csharp_property_name, "Value");
                assert_eq!(fields[0].json_name, "Value");
                assert!(!fields[0].is_optional);
            }
            _ => panic!("expected Record kind"),
        }
    }

    #[test]
    fn newtype_optional_inner_type() {
        let input: DeriveInput = parse_quote! { struct MaybeId(Option<String>); };
        let ir = process_newtype(&input);
        match &ir.kind {
            DerivedCSharpKind::Record(fields) => {
                assert!(
                    fields[0].is_optional,
                    "Option inner type should be optional"
                );
            }
            _ => panic!("expected Record kind"),
        }
    }

    #[test]
    fn transparent_newtype_sets_flag() {
        let input: DeriveInput = parse_quote! {
            #[serde(transparent)]
            struct UserId(String);
        };
        let ir = process_newtype(&input);
        assert!(ir.transparent, "transparent newtype should set flag");
    }

    #[test]
    fn non_transparent_newtype_flag_false() {
        let input: DeriveInput = parse_quote! { struct UserId(String); };
        let ir = process_newtype(&input);
        assert!(!ir.transparent);
    }
}
