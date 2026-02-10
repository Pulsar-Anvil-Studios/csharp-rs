// Rust guideline compliant 2026-02-10
//! Container-level attribute parsing for `#[serde(...)]` and `#[csharp(...)]`.
//!
//! Extracts `rename_all` from serde attributes and `namespace`, `export`,
//! `export_to` from csharp attributes on the container (struct/enum).

use super::Inflection;
use crate::config::CSharpNamespace;
use syn::{Attribute, Lit};

/// Parsed container-level attributes.
#[derive(Debug, Default)]
pub struct ContainerAttr {
    /// The serde `rename_all` inflection, if specified.
    pub rename_all: Option<Inflection>,
    /// C# namespace override from `#[csharp(namespace = "...")]`.
    pub namespace: Option<String>,
    /// Whether `#[csharp(export)]` was specified.
    pub export: bool,
    /// Custom export path from `#[csharp(export_to = "...")]`.
    pub export_to: Option<String>,
}

impl ContainerAttr {
    /// Parses container attributes from a slice of `syn::Attribute`.
    ///
    /// # Errors
    ///
    /// Returns a `syn::Error` if attribute syntax is invalid.
    pub fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut result = Self::default();

        for attr in attrs {
            if attr.path().is_ident("serde") {
                result.parse_serde(attr)?;
            } else if attr.path().is_ident("csharp") {
                result.parse_csharp(attr)?;
            }
        }

        Ok(result)
    }

    fn parse_serde(&mut self, attr: &Attribute) -> syn::Result<()> {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(lit_str) = lit {
                    self.rename_all = Inflection::from_rename_all(&lit_str.value());
                }
            }
            // Silently ignore other serde attributes (handled by serde itself).
            Ok(())
        })
    }

    fn parse_csharp(&mut self, attr: &Attribute) -> syn::Result<()> {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("namespace") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(ref lit_str) = lit {
                    let raw = lit_str.value();
                    CSharpNamespace::new(raw.as_str()).map_err(|msg| {
                        meta.error(format!("invalid namespace \"{raw}\": {msg}"))
                    })?;
                    self.namespace = Some(raw);
                }
                return Ok(());
            }

            if meta.path.is_ident("export") {
                self.export = true;
                return Ok(());
            }

            if meta.path.is_ident("export_to") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(lit_str) = lit {
                    self.export_to = Some(lit_str.value());
                    self.export = true;
                }
                return Ok(());
            }

            Err(meta.error("unrecognized csharp attribute"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn parse_serde_rename_all() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(rename_all = "camelCase")])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.rename_all, Some(Inflection::Camel));
    }

    #[test]
    fn parse_csharp_namespace() {
        let attrs: Vec<Attribute> =
            vec![parse_quote!(#[csharp(namespace = "PulsarAnvil.Types")])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.namespace.as_deref(), Some("PulsarAnvil.Types"));
    }

    #[test]
    fn parse_csharp_export() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[csharp(export)])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert!(container.export);
    }

    #[test]
    fn parse_csharp_export_to() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[csharp(export_to = "out/types")])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert!(container.export);
        assert_eq!(container.export_to.as_deref(), Some("out/types"));
    }

    #[test]
    fn parse_combined_attrs() {
        let attrs: Vec<Attribute> = vec![
            parse_quote!(#[serde(rename_all = "PascalCase")]),
            parse_quote!(#[csharp(namespace = "Game", export)]),
        ];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.rename_all, Some(Inflection::Pascal));
        assert_eq!(container.namespace.as_deref(), Some("Game"));
        assert!(container.export);
    }

    #[test]
    fn no_attrs_returns_defaults() {
        let attrs: Vec<Attribute> = vec![];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.rename_all, None);
        assert_eq!(container.namespace, None);
        assert!(!container.export);
        assert_eq!(container.export_to, None);
    }

    #[test]
    fn unrecognized_csharp_attr_errors() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[csharp(bogus)])];
        let result = ContainerAttr::from_attrs(&attrs);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_namespace_errors_at_parse() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[csharp(namespace = "123bad")])];
        let result = ContainerAttr::from_attrs(&attrs);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid namespace"),
            "error should mention invalid namespace: {err}"
        );
    }

    #[test]
    fn empty_namespace_errors_at_parse() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[csharp(namespace = "")])];
        let result = ContainerAttr::from_attrs(&attrs);
        assert!(result.is_err());
    }
}
