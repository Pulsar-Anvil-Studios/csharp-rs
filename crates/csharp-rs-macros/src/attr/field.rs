// Rust guideline compliant 2026-02-10
//! Field-level attribute parsing for `#[serde(...)]`.
//!
//! Supports `#[serde(rename = "...")]`, `#[serde(skip)]`,
//! `#[serde(skip_serializing)]`, and `#[serde(skip_serializing_if = "...")]`.

use syn::{Attribute, Lit};

/// Parsed field-level serde attributes.
#[derive(Debug, Default)]
pub struct FieldAttr {
    /// Per-field JSON name override from `#[serde(rename = "...")]`.
    pub rename: Option<String>,
    /// Field excluded from C# output (`serde(skip)` or `serde(skip_serializing)`).
    pub skip: bool,
    /// Field may be absent in JSON (`serde(skip_serializing_if = "...")`),
    /// rendered as nullable (`T?`) in C#.
    pub skip_serializing_if: bool,
}

impl FieldAttr {
    /// Parses field attributes from a slice of `syn::Attribute`.
    ///
    /// # Errors
    ///
    /// Returns a `syn::Error` if attribute syntax is invalid.
    pub fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut result = Self::default();

        for attr in attrs {
            if attr.path().is_ident("serde") {
                result.parse_serde(attr)?;
            }
        }

        Ok(result)
    }

    fn parse_serde(&mut self, attr: &Attribute) -> syn::Result<()> {
        attr.parse_nested_meta(|meta| {
            let ident = meta.path.get_ident().map(ToString::to_string);
            match ident.as_deref() {
                Some("rename") => {
                    let value = meta.value()?;
                    let lit: Lit = value.parse()?;
                    if let Lit::Str(lit_str) = lit {
                        self.rename = Some(lit_str.value());
                    }
                }
                Some("skip" | "skip_serializing") => self.skip = true,
                Some("skip_serializing_if") => {
                    // Consume the value (predicate path) but only track the flag.
                    let value = meta.value()?;
                    let _lit: Lit = value.parse()?;
                    self.skip_serializing_if = true;
                }
                Some("skip_deserializing") => {
                    // Consume optional value; field stays in C# output.
                    let _ = meta.value().and_then(syn::parse::ParseBuffer::parse::<Lit>);
                }
                // Silently ignore other serde attributes (handled by serde itself).
                _ => {}
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn parse_serde_rename() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(rename = "userId")])];
        let field_attr = FieldAttr::from_attrs(&attrs).unwrap();
        assert_eq!(field_attr.rename.as_deref(), Some("userId"));
    }

    #[test]
    fn parse_serde_skip() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(skip)])];
        let field_attr = FieldAttr::from_attrs(&attrs).unwrap();
        assert!(field_attr.skip);
    }

    #[test]
    fn parse_serde_skip_serializing() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(skip_serializing)])];
        let field_attr = FieldAttr::from_attrs(&attrs).unwrap();
        assert!(field_attr.skip);
    }

    #[test]
    fn parse_serde_skip_serializing_if() {
        let attrs: Vec<Attribute> =
            vec![parse_quote!(#[serde(skip_serializing_if = "Option::is_none")])];
        let field_attr = FieldAttr::from_attrs(&attrs).unwrap();
        assert!(field_attr.skip_serializing_if);
        assert!(!field_attr.skip);
    }

    #[test]
    fn parse_serde_skip_deserializing_ignored() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(skip_deserializing)])];
        let field_attr = FieldAttr::from_attrs(&attrs).unwrap();
        assert!(!field_attr.skip);
        assert!(!field_attr.skip_serializing_if);
        assert!(field_attr.rename.is_none());
    }

    #[test]
    fn no_attrs_returns_defaults() {
        let attrs: Vec<Attribute> = vec![];
        let field_attr = FieldAttr::from_attrs(&attrs).unwrap();
        assert!(field_attr.rename.is_none());
        assert!(!field_attr.skip);
        assert!(!field_attr.skip_serializing_if);
    }

    #[test]
    fn parse_combined_rename_and_skip_serializing_if() {
        let attrs: Vec<Attribute> =
            vec![parse_quote!(#[serde(rename = "hp", skip_serializing_if = "is_zero")])];
        let field_attr = FieldAttr::from_attrs(&attrs).unwrap();
        assert_eq!(field_attr.rename.as_deref(), Some("hp"));
        assert!(field_attr.skip_serializing_if);
    }

    #[test]
    fn unknown_serde_attrs_ignored() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(default)])];
        let field_attr = FieldAttr::from_attrs(&attrs).unwrap();
        assert!(field_attr.rename.is_none());
        assert!(!field_attr.skip);
        assert!(!field_attr.skip_serializing_if);
    }
}
