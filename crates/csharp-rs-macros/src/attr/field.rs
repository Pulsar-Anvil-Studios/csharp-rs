// Rust guideline compliant 2026-02-10
//! Field-level attribute parsing (stub for Step 1).
//!
//! Will support `#[serde(rename = "...")]`, `#[serde(skip)]`, and
//! `#[serde(flatten)]` in future steps.

use syn::Attribute;

/// Parsed field-level attributes (stub).
#[derive(Debug, Default)]
pub struct FieldAttr;

impl FieldAttr {
    /// Parses field attributes from a slice of `syn::Attribute`.
    ///
    /// Currently a no-op stub; returns defaults.
    ///
    /// # Errors
    ///
    /// Returns a `syn::Error` if attribute syntax is invalid.
    #[expect(clippy::unnecessary_wraps, reason = "will parse attrs in Step 3")]
    pub fn from_attrs(_attrs: &[Attribute]) -> syn::Result<Self> {
        Ok(Self)
    }
}
