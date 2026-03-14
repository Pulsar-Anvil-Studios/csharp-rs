// Rust guideline compliant 2026-03-14
//! Namespace validation for `#[csharp(namespace = "...")]` attribute parsing.

use std::fmt;

/// A validated C# namespace (e.g., `"MyApp.Types"`).
///
/// Each dot-separated segment must be a valid C# identifier:
/// starts with a letter or underscore, followed by alphanumerics or
/// underscores. The namespace must be non-empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CSharpNamespace(String);

impl CSharpNamespace {
    /// Creates a new validated namespace.
    ///
    /// # Errors
    ///
    /// Returns an error message if the namespace is empty, contains
    /// empty segments, or has segments that are not valid C# identifiers.
    pub fn new(s: impl Into<String>) -> Result<Self, &'static str> {
        let value = s.into();
        validate_namespace(&value)?;
        Ok(Self(value))
    }
}

impl fmt::Display for CSharpNamespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for CSharpNamespace {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<&str> for CSharpNamespace {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Validates that `s` is a well-formed C# namespace.
fn validate_namespace(s: &str) -> Result<(), &'static str> {
    if s.is_empty() {
        return Err("namespace must not be empty");
    }

    for segment in s.split('.') {
        if segment.is_empty() {
            return Err("namespace must not contain empty segments");
        }

        let mut chars = segment.chars();
        let first = chars.next().expect("segment is non-empty");
        if !first.is_ascii_alphabetic() && first != '_' {
            return Err("namespace segment must start with a letter or underscore");
        }

        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("namespace segment must contain only alphanumerics and underscores");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_valid_single_segment() {
        assert!(CSharpNamespace::new("Foo").is_ok());
        assert!(CSharpNamespace::new("_Foo").is_ok());
        assert!(CSharpNamespace::new("Foo123").is_ok());
        assert!(CSharpNamespace::new("Generated").is_ok());
    }

    #[test]
    fn namespace_valid_multi_segment() {
        assert!(CSharpNamespace::new("Foo.Bar").is_ok());
        assert!(CSharpNamespace::new("PulsarAnvil.Types").is_ok());
        assert!(CSharpNamespace::new("My.Deep.Nested.Ns").is_ok());
    }

    #[test]
    fn namespace_invalid_empty() {
        assert!(CSharpNamespace::new("").is_err());
    }

    #[test]
    fn namespace_invalid_empty_segment() {
        assert!(CSharpNamespace::new(".Foo").is_err());
        assert!(CSharpNamespace::new("Foo.").is_err());
        assert!(CSharpNamespace::new("Foo..Bar").is_err());
    }

    #[test]
    fn namespace_invalid_starts_with_digit() {
        assert!(CSharpNamespace::new("123foo").is_err());
        assert!(CSharpNamespace::new("Foo.1Bar").is_err());
    }

    #[test]
    fn namespace_invalid_special_chars() {
        assert!(CSharpNamespace::new("foo bar").is_err());
        assert!(CSharpNamespace::new("foo-bar").is_err());
    }

    #[test]
    fn namespace_display() {
        let ns = CSharpNamespace::new("PulsarAnvil.Types").unwrap();
        assert_eq!(ns.to_string(), "PulsarAnvil.Types");
    }

    #[test]
    fn namespace_as_ref() {
        let ns = CSharpNamespace::new("Game").unwrap();
        let s: &str = ns.as_ref();
        assert_eq!(s, "Game");
    }

    #[test]
    fn namespace_partial_eq_str() {
        let ns = CSharpNamespace::new("Generated").unwrap();
        assert_eq!(ns, "Generated");
    }
}
