// Rust guideline compliant 2026-02-14
//! Type dispatch and intermediate representation for C# code generation.
//!
//! Dispatches `syn::DeriveInput` to the appropriate handler based on the
//! Rust data structure kind (struct with named fields, enum, etc.).

pub mod named;
pub mod simple_enum;
pub mod tagged_enum;

use crate::attr::container::ContainerAttr;
use crate::config::{CSharpConfig, CSharpNamespace};
use proc_macro2::{Ident, TokenStream};
use syn::{Data, DataStruct, DeriveInput, Fields};

/// How a field is flattened into the parent record.
#[derive(Debug)]
pub enum FlattenKind {
    /// Normal field (not flattened).
    None,
    /// Struct flatten: inline the inner type's properties.
    Struct,
    /// `HashMap` flatten: emit `[JsonExtensionData]` property.
    HashMap {
        /// Token stream for `<K as CSharp>::csharp_name()`.
        key_expr: TokenStream,
        /// Token stream for `<V as CSharp>::csharp_name()`.
        value_expr: TokenStream,
    },
}

/// A single field in a C# record or tagged enum struct variant.
///
/// # Flatten invariants
///
/// When `flatten` is [`FlattenKind::None`], all fields are meaningful.
///
/// When `flatten` is [`FlattenKind::Struct`], only `type_expr` is meaningful
/// (it evaluates to the inner type's `csharp_fields()` call). The
/// `csharp_property_name`, `json_name` fields are empty strings, and
/// `is_optional` is `false`.
///
/// When `flatten` is [`FlattenKind::HashMap`], none of the standard fields
/// are meaningful — the key/value types live inside the `FlattenKind` variant.
/// All string fields are empty, `type_expr` is an empty `TokenStream`, and
/// `is_optional` is `false`.
#[derive(Debug)]
pub struct CSharpField {
    /// C# `PascalCase` property name (empty for flatten fields).
    pub csharp_property_name: String,
    /// JSON wire name for serialization attributes (empty for flatten fields).
    pub json_name: String,
    /// Expression evaluating to the C# type name at consumer compile time.
    ///
    /// For `FlattenKind::None`: evaluates to `CSharp::csharp_name()`.
    /// For `FlattenKind::Struct`: evaluates to `CSharp::csharp_fields()`.
    /// For `FlattenKind::HashMap`: empty `TokenStream` (unused).
    pub type_expr: TokenStream,
    /// Whether this field is `Option<T>` (always `false` for flatten fields).
    pub is_optional: bool,
    /// How this field participates in flatten inlining.
    pub flatten: FlattenKind,
}

/// A C# enum variant in the intermediate representation.
#[derive(Debug)]
pub struct CSharpVariant {
    /// C# variant name (used as-is from the Rust variant, already `PascalCase`).
    pub csharp_name: String,
    /// JSON serialization name (after `rename_all` or per-variant `rename`).
    pub json_name: String,
}

/// How the enum is tagged in JSON (from serde attributes).
#[derive(Debug)]
pub enum EnumTagging {
    /// Default serde: `{"VariantName": data}` / `"UnitVariant"`.
    External,
    /// `#[serde(tag = "...")]`: discriminator merged into object.
    Internal { tag: String },
    /// `#[serde(tag = "...", content = "...")]`: separate tag and content keys.
    Adjacent { tag: String, content: String },
    /// `#[serde(untagged)]`: no discriminator, try each variant.
    Untagged,
}

/// Data carried by a variant in a tagged enum.
#[derive(Debug)]
pub enum TaggedVariantData {
    /// No data (unit variant).
    Unit,
    /// Wraps a single type: `Variant(Type)`.
    Newtype { type_expr: TokenStream },
    /// Named fields: `Variant { field: Type, ... }`.
    Struct(Vec<CSharpField>),
}

/// A variant in a tagged enum.
#[derive(Debug)]
pub struct TaggedVariant {
    /// C# record name (`PascalCase`, from Rust variant ident).
    pub csharp_name: String,
    /// JSON discriminator value (after `rename_all` / per-variant `rename`).
    pub json_name: String,
    /// Data carried by this variant.
    pub data: TaggedVariantData,
}

/// The kind of C# type being generated.
#[derive(Debug)]
pub enum DerivedCSharpKind {
    /// A `sealed record` with properties (from a Rust struct with named fields).
    Record(Vec<CSharpField>),
    /// A `public enum` with unit variants (from a Rust enum).
    Enum(Vec<CSharpVariant>),
    /// A tagged enum hierarchy (from a Rust enum with data variants).
    TaggedEnum {
        /// The tagging strategy derived from serde attributes.
        tagging: EnumTagging,
        /// The variants with their data payloads.
        variants: Vec<TaggedVariant>,
    },
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
    /// The kind of C# type and its contents.
    pub kind: DerivedCSharpKind,
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

        Data::Enum(enum_data) => {
            let has_data_variants = enum_data.variants.iter().any(|v| !v.fields.is_empty());
            let has_explicit_tagging =
                container.tag.is_some() || container.content.is_some() || container.untagged;

            if has_data_variants || has_explicit_tagging {
                tagged_enum::tagged_enum(input, enum_data, &container, config)
            } else {
                simple_enum::simple_enum(input, enum_data, &container, config)
            }
        }

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
        assert!(
            matches!(&ir.kind, DerivedCSharpKind::Record(fields) if fields.len() == 1),
            "expected Record kind with 1 field"
        );
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
    fn unit_enum_succeeds() {
        let input: DeriveInput = parse_quote! {
            enum Color {
                Red,
                Green,
                Blue,
            }
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_ok());
        let ir = result.unwrap();
        assert_eq!(ir.csharp_name, "Color");
        assert!(
            matches!(&ir.kind, DerivedCSharpKind::Enum(variants) if variants.len() == 3),
            "expected Enum kind with 3 variants"
        );
    }

    #[test]
    fn enum_with_struct_variant_and_tag_succeeds() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Message {
                Request { id: String },
                Quit,
            }
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_ok());
        let ir = result.unwrap();
        assert_eq!(ir.csharp_name, "Message");
        assert!(matches!(ir.kind, DerivedCSharpKind::TaggedEnum { .. }));
    }

    #[test]
    fn enum_with_data_variant_no_tag_defaults_to_external() {
        let input: DeriveInput = parse_quote! {
            enum Message {
                Text(String),
                Quit,
            }
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_ok());
        match &result.unwrap().kind {
            DerivedCSharpKind::TaggedEnum { tagging, .. } => {
                assert!(matches!(tagging, EnumTagging::External));
            }
            _ => panic!("expected TaggedEnum kind"),
        }
    }

    #[test]
    fn enum_with_multi_field_tuple_variant_errors() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Message {
                Data(String, i32),
            }
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("tuple variants"),
            "error should mention tuple variants: {err}"
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

    #[test]
    fn all_unit_with_tag_becomes_tagged_enum() {
        let input: DeriveInput = parse_quote! {
            #[serde(tag = "type")]
            enum Status {
                Active,
                Inactive,
            }
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_ok());
        assert!(matches!(
            result.unwrap().kind,
            DerivedCSharpKind::TaggedEnum { .. }
        ));
    }

    #[test]
    fn all_unit_without_tag_stays_simple_enum() {
        let input: DeriveInput = parse_quote! {
            enum Color {
                Red,
                Green,
            }
        };
        let result = process_input(&input, &default_config());
        assert!(result.is_ok());
        assert!(matches!(result.unwrap().kind, DerivedCSharpKind::Enum(_)));
    }
}
