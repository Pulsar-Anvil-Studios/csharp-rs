// Rust guideline compliant 2026-03-15
//! Container-level attribute parsing for `#[serde(...)]` and `#[csharp(...)]`.
//!
//! Extracts `rename_all` from serde attributes and `namespace`, `export`,
//! `export_to`, `concrete`, `bound` from csharp attributes on the container
//! (struct/enum).

use std::collections::HashMap;

use super::Inflection;
use crate::config::CSharpNamespace;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Attribute, Ident, Lit, Token, WherePredicate};

/// A single `Ident = Type` mapping inside `#[csharp(concrete(...))]`.
struct ConcreteMapping {
    ident: Ident,
    _eq: Token![=],
    ty: syn::Type,
}

impl Parse for ConcreteMapping {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            ident: input.parse()?,
            _eq: input.parse()?,
            ty: input.parse()?,
        })
    }
}

/// Parsed container-level attributes.
#[derive(Debug, Default)]
pub struct ContainerAttr {
    /// The serde `rename_all` inflection for variant names, if specified.
    pub rename_all: Option<Inflection>,
    /// The serde `rename_all_fields` inflection for fields within enum variants, if specified.
    pub rename_all_fields: Option<Inflection>,
    /// The serde internally-tagged discriminant field name from `#[serde(tag = "...")]`.
    pub tag: Option<String>,
    /// The serde adjacently-tagged content field name from `#[serde(content = "...")]`.
    pub content: Option<String>,
    /// Whether the enum is untagged via `#[serde(untagged)]`.
    pub untagged: bool,
    /// Whether the struct is transparent via `#[serde(transparent)]`.
    pub transparent: bool,
    /// C# namespace override from `#[csharp(namespace = "...")]`.
    pub namespace: Option<String>,
    /// Whether `#[csharp(export)]` was specified.
    pub export: bool,
    /// Custom export path from `#[csharp(export_to = "...")]`.
    pub export_to: Option<String>,
    /// Concrete type substitutions from `#[csharp(concrete(T = String, ...))]`.
    pub concrete: HashMap<Ident, syn::Type>,
    /// Custom where-clause bounds from `#[csharp(bound = "...")]`.
    pub bound: Option<Vec<WherePredicate>>,
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
            } else if meta.path.is_ident("rename_all_fields") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(lit_str) = lit {
                    self.rename_all_fields = Inflection::from_rename_all(&lit_str.value());
                }
            } else if meta.path.is_ident("tag") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(lit_str) = lit {
                    self.tag = Some(lit_str.value());
                }
            } else if meta.path.is_ident("content") {
                let value = meta.value()?;
                let lit: Lit = value.parse()?;
                if let Lit::Str(lit_str) = lit {
                    self.content = Some(lit_str.value());
                }
            } else if meta.path.is_ident("untagged") {
                self.untagged = true;
            } else if meta.path.is_ident("transparent") {
                self.transparent = true;
            }
            // Silently ignore other serde attributes (handled by serde itself).
            Ok(())
        })
    }

    fn parse_csharp(&mut self, attr: &Attribute) -> syn::Result<()> {
        attr.parse_nested_meta(|meta| {
            let ident = meta.path.get_ident().map(ToString::to_string);
            match ident.as_deref() {
                Some("namespace") => {
                    let value = meta.value()?;
                    let lit: Lit = value.parse()?;
                    if let Lit::Str(ref lit_str) = lit {
                        let raw = lit_str.value();
                        CSharpNamespace::new(raw.as_str()).map_err(|msg| {
                            meta.error(format!("invalid namespace \"{raw}\": {msg}"))
                        })?;
                        self.namespace = Some(raw);
                    }
                }
                Some("export") => self.export = true,
                Some("export_to") => {
                    let value = meta.value()?;
                    let lit: Lit = value.parse()?;
                    if let Lit::Str(lit_str) = lit {
                        self.export_to = Some(lit_str.value());
                        self.export = true;
                    }
                }
                Some("concrete") => {
                    let content;
                    syn::parenthesized!(content in meta.input);
                    let pairs =
                        Punctuated::<ConcreteMapping, Token![,]>::parse_terminated(&content)?;
                    for mapping in pairs {
                        self.concrete.insert(mapping.ident, mapping.ty);
                    }
                }
                Some("bound") => {
                    let value = meta.value()?;
                    let lit: Lit = value.parse()?;
                    if let Lit::Str(lit_str) = lit {
                        let predicates: Punctuated<WherePredicate, Token![,]> =
                            lit_str.parse_with(Punctuated::parse_terminated)?;
                        self.bound = Some(predicates.into_iter().collect());
                    }
                }
                _ => return Err(meta.error("unrecognized csharp attribute")),
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
    fn parse_serde_rename_all() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(rename_all = "camelCase")])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.rename_all, Some(Inflection::Camel));
    }

    #[test]
    fn parse_csharp_namespace() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[csharp(namespace = "PulsarAnvil.Types")])];
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
        assert!(!container.transparent);
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

    #[test]
    fn parse_serde_tag() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(tag = "type")])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.tag.as_deref(), Some("type"));
    }

    #[test]
    fn parse_serde_tag_and_content() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(tag = "t", content = "c")])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.tag.as_deref(), Some("t"));
        assert_eq!(container.content.as_deref(), Some("c"));
    }

    #[test]
    fn parse_serde_untagged() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(untagged)])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert!(container.untagged);
    }

    #[test]
    fn parse_combined_tag_and_rename_all() {
        let attrs: Vec<Attribute> =
            vec![parse_quote!(#[serde(tag = "kind", rename_all = "camelCase")])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.tag.as_deref(), Some("kind"));
        assert_eq!(container.rename_all, Some(Inflection::Camel));
    }

    #[test]
    fn no_tag_defaults_to_none() {
        let attrs: Vec<Attribute> = vec![];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert!(container.tag.is_none());
        assert!(container.content.is_none());
        assert!(!container.untagged);
    }

    #[test]
    fn parse_serde_rename_all_fields() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(rename_all_fields = "camelCase")])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.rename_all_fields, Some(Inflection::Camel));
    }

    #[test]
    fn rename_all_and_rename_all_fields_independent() {
        let attrs: Vec<Attribute> =
            vec![parse_quote!(#[serde(rename_all = "UPPERCASE", rename_all_fields = "camelCase")])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.rename_all, Some(Inflection::Upper));
        assert_eq!(container.rename_all_fields, Some(Inflection::Camel));
    }

    #[test]
    fn parse_serde_transparent() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[serde(transparent)])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert!(container.transparent);
    }

    #[test]
    fn transparent_with_namespace() {
        let attrs: Vec<Attribute> = vec![
            parse_quote!(#[serde(transparent)]),
            parse_quote!(#[csharp(namespace = "Ids")]),
        ];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert!(container.transparent);
        assert_eq!(container.namespace.as_deref(), Some("Ids"));
    }

    #[test]
    fn parse_csharp_concrete_single() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[csharp(concrete(T = String))])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.concrete.len(), 1);
        assert!(container.concrete.contains_key(&parse_quote!(T)));
    }

    #[test]
    fn parse_csharp_concrete_multiple() {
        let attrs: Vec<Attribute> =
            vec![parse_quote!(#[csharp(concrete(T = String, U = i32))])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.concrete.len(), 2);
        assert!(container.concrete.contains_key(&parse_quote!(T)));
        assert!(container.concrete.contains_key(&parse_quote!(U)));
    }

    #[test]
    fn parse_csharp_bound() {
        let attrs: Vec<Attribute> =
            vec![parse_quote!(#[csharp(bound = "T: std::fmt::Display")])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert!(container.bound.is_some());
        assert_eq!(container.bound.unwrap().len(), 1);
    }

    #[test]
    fn parse_csharp_concrete_with_export() {
        let attrs: Vec<Attribute> =
            vec![parse_quote!(#[csharp(concrete(T = String), export)])];
        let container = ContainerAttr::from_attrs(&attrs).unwrap();
        assert_eq!(container.concrete.len(), 1);
        assert!(container.export);
    }
}
