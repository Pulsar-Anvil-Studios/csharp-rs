// Rust guideline compliant 2026-02-10
//! Tagged enum processing for C# code generation.
//!
//! Converts a Rust enum with data variants into the [`DerivedCSharp`]
//! intermediate representation for tagged C# type hierarchies.

use crate::attr::container::ContainerAttr;
use crate::config::CSharpConfig;
use crate::types::DerivedCSharp;
use syn::{DataEnum, DeriveInput};

/// Builds a [`DerivedCSharp`] from an enum with data variants.
///
/// # Errors
///
/// Returns a `syn::Error` if variant types are unsupported or attributes invalid.
pub fn tagged_enum(
    _input: &DeriveInput,
    _enum_data: &DataEnum,
    _container: &ContainerAttr,
    _config: &CSharpConfig,
) -> syn::Result<DerivedCSharp> {
    todo!("tagged_enum IR builder \u{2014} implemented in Task 3")
}
