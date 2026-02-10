// Rust guideline compliant 2026-02-10
//! Type dispatch and intermediate representation for C# code generation.
//!
//! Dispatches `syn::DeriveInput` to the appropriate handler based on the
//! Rust data structure kind (struct with named fields, enum, etc.).

pub mod named;

use crate::attr::container::ContainerAttr;
use crate::config::{CSharpConfig, CSharpNamespace};
use proc_macro2::{Ident, TokenStream};
use syn::{Data, DataStruct, DeriveInput, Fields};

/// A C# field in the intermediate representation.
#[derive(Debug)]
pub struct CSharpField {
    /// C# property name (`PascalCase`).
    pub csharp_property_name: String,
    /// JSON serialization name (after `rename_all`).
    pub json_name: String,
    /// Token stream that evaluates to the C# type name at compile time.
    pub type_expr: TokenStream,
    /// Whether this field is `Option<T>` (nullable in C#).
    pub is_optional: bool,
}

/// Intermediate representation for a derived C# type.
#[derive(Debug)]
pub struct DerivedCSharp {
    /// The Rust type identifier.
    pub rust_ident: Ident,
    /// The C# type name.
    pub csharp_name: String,
    /// C# namespace (from config or attribute override).
    pub namespace: CSharpNamespace,
    /// The fields of the C# type.
    pub fields: Vec<CSharpField>,
    /// Whether to generate an export test.
    pub export: bool,
    /// Custom export path (overrides config default).
    pub export_to: Option<String>,
}

/// Processes a `DeriveInput` into a [`DerivedCSharp`] IR.
///
/// # Errors
///
/// Returns a `syn::Error` for unsupported data structures.
pub fn process_input(input: &DeriveInput, config: &CSharpConfig) -> syn::Result<DerivedCSharp> {
    let container = ContainerAttr::from_attrs(&input.attrs)?;

    match &input.data {
        Data::Struct(DataStruct {
            fields: Fields::Named(named),
            ..
        }) => named::named_struct(input, named, &container, config),

        Data::Struct(DataStruct {
            fields: Fields::Unnamed(_),
            ..
        }) => Err(syn::Error::new_spanned(
            &input.ident,
            "csharp-rs: tuple structs are not yet supported",
        )),

        Data::Struct(DataStruct {
            fields: Fields::Unit,
            ..
        }) => Err(syn::Error::new_spanned(
            &input.ident,
            "csharp-rs: unit structs are not yet supported",
        )),

        Data::Enum(_) => Err(syn::Error::new_spanned(
            &input.ident,
            "csharp-rs: enums are not yet supported",
        )),

        Data::Union(_) => Err(syn::Error::new_spanned(
            &input.ident,
            "csharp-rs: unions are not supported",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn default_config() -> CSharpConfig {
        CSharpConfig::default()
    }

    #[test]
    fn named_struct_succeeds() {
        let input: DeriveInput = parse_quote! {
            struct Foo {
                bar: String,
            }
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_ok());
        let ir = result.unwrap();
        assert_eq!(ir.csharp_name, "Foo");
        assert_eq!(ir.fields.len(), 1);
    }

    #[test]
    fn tuple_struct_errors() {
        let input: DeriveInput = parse_quote! {
            struct Wrapper(String);
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("tuple structs"),
            "error should mention tuple structs: {err}"
        );
    }

    #[test]
    fn unit_struct_errors() {
        let input: DeriveInput = parse_quote! {
            struct Unit;
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unit structs"),
            "error should mention unit structs: {err}"
        );
    }

    #[test]
    fn enum_errors() {
        let input: DeriveInput = parse_quote! {
            enum Color {
                Red,
                Green,
                Blue,
            }
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("enums are not yet supported"),
            "error should mention enums: {err}"
        );
    }

    #[test]
    fn union_errors() {
        let input: DeriveInput = parse_quote! {
            union MyUnion {
                i: i32,
                f: f32,
            }
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unions are not supported"),
            "error should mention unions: {err}"
        );
    }
}
