// Rust guideline compliant 2026-02-10
//! Compile-time configuration from `Cargo.toml` metadata.
//!
//! Reads `[package.metadata.csharp]` from the consumer's `Cargo.toml`
//! (located via `CARGO_MANIFEST_DIR`) and provides defaults when keys
//! are absent.

use std::fmt;
use std::path::{Path, PathBuf};

/// Which JSON serializer attributes to emit in generated C#.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Serializer {
    /// `System.Text.Json` attributes (`[JsonPropertyName]`).
    SystemTextJson,
    /// `Newtonsoft.Json` attributes (`[JsonProperty]`).
    Newtonsoft,
}

/// Target C# language version for code generation.
///
/// Ordered by version number so comparisons like `>= CSharp10` work
/// for feature-gating in codegen (e.g., file-scoped namespaces).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CSharpVersion {
    /// C# 9.0 — `record`, `init` (baseline).
    #[default]
    CSharp9,
    /// C# 10.0 — file-scoped namespaces.
    CSharp10,
    /// C# 11.0 — `required` keyword.
    CSharp11,
    /// C# 12.0 — primary constructors.
    CSharp12,
}

impl CSharpVersion {
    /// Parses a version string like `"9.0"` into a [`CSharpVersion`].
    ///
    /// Returns `None` for unrecognized values.
    pub fn from_version_str(s: &str) -> Option<Self> {
        match s {
            "9.0" => Some(Self::CSharp9),
            "10.0" => Some(Self::CSharp10),
            "11.0" => Some(Self::CSharp11),
            "12.0" => Some(Self::CSharp12),
            _ => None,
        }
    }
}

impl fmt::Display for CSharpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::CSharp9 => "9.0",
            Self::CSharp10 => "10.0",
            Self::CSharp11 => "11.0",
            Self::CSharp12 => "12.0",
        };
        f.write_str(s)
    }
}

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

/// Compile-time configuration for C# code generation.
#[derive(Debug, Clone)]
pub struct CSharpConfig {
    /// Default C# namespace (e.g., `"MyApp.Types"`).
    pub namespace: CSharpNamespace,
    /// Which serializer attributes to emit.
    pub serializer: Serializer,
    /// Target C# language version.
    pub target: CSharpVersion,
    /// Output directory for exported `.cs` files.
    pub export_dir: PathBuf,
}

impl Default for CSharpConfig {
    fn default() -> Self {
        Self {
            namespace: CSharpNamespace::new("Generated")
                .expect("default namespace is valid"),
            serializer: Serializer::SystemTextJson,
            target: CSharpVersion::default(),
            export_dir: PathBuf::from("./csharp-bindings"),
        }
    }
}

impl CSharpConfig {
    /// Loads configuration from `CARGO_MANIFEST_DIR/Cargo.toml`.
    ///
    /// Falls back to defaults for any missing keys.
    pub fn from_manifest_dir(manifest_dir: &Path) -> Self {
        let cargo_toml_path = manifest_dir.join("Cargo.toml");
        let Ok(content) = std::fs::read_to_string(&cargo_toml_path) else {
            return Self::default();
        };
        Self::from_toml_str(&content)
    }

    /// Parses configuration from a TOML string.
    ///
    /// Falls back to defaults for any missing keys.
    pub fn from_toml_str(content: &str) -> Self {
        let table: toml::Table = match content.parse() {
            Ok(t) => t,
            Err(_) => return Self::default(),
        };

        let metadata = table
            .get("package")
            .and_then(toml::Value::as_table)
            .and_then(|pkg| pkg.get("metadata"))
            .and_then(toml::Value::as_table)
            .and_then(|meta| meta.get("csharp"))
            .and_then(toml::Value::as_table);

        let Some(csharp) = metadata else {
            return Self::default();
        };

        let defaults = Self::default();

        let namespace = csharp
            .get("namespace")
            .and_then(toml::Value::as_str)
            .and_then(|s| CSharpNamespace::new(s).ok())
            .unwrap_or(defaults.namespace);

        let serializer = csharp
            .get("serializer")
            .and_then(toml::Value::as_str)
            .map_or(defaults.serializer, |s| match s {
                "newtonsoft" => Serializer::Newtonsoft,
                _ => Serializer::SystemTextJson,
            });

        let target = csharp
            .get("target")
            .and_then(toml::Value::as_str)
            .and_then(CSharpVersion::from_version_str)
            .unwrap_or(defaults.target);

        let export_dir = csharp
            .get("export-dir")
            .and_then(toml::Value::as_str)
            .map_or(defaults.export_dir, PathBuf::from);

        Self {
            namespace,
            serializer,
            target,
            export_dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- CSharpVersion tests ---

    #[test]
    fn version_parse_valid() {
        assert_eq!(
            CSharpVersion::from_version_str("9.0"),
            Some(CSharpVersion::CSharp9)
        );
        assert_eq!(
            CSharpVersion::from_version_str("10.0"),
            Some(CSharpVersion::CSharp10)
        );
        assert_eq!(
            CSharpVersion::from_version_str("11.0"),
            Some(CSharpVersion::CSharp11)
        );
        assert_eq!(
            CSharpVersion::from_version_str("12.0"),
            Some(CSharpVersion::CSharp12)
        );
    }

    #[test]
    fn version_parse_invalid_returns_none() {
        assert_eq!(CSharpVersion::from_version_str("8.0"), None);
        assert_eq!(CSharpVersion::from_version_str("13.0"), None);
        assert_eq!(CSharpVersion::from_version_str(""), None);
        assert_eq!(CSharpVersion::from_version_str("latest"), None);
    }

    #[test]
    fn version_display() {
        assert_eq!(CSharpVersion::CSharp9.to_string(), "9.0");
        assert_eq!(CSharpVersion::CSharp10.to_string(), "10.0");
        assert_eq!(CSharpVersion::CSharp11.to_string(), "11.0");
        assert_eq!(CSharpVersion::CSharp12.to_string(), "12.0");
    }

    #[test]
    fn version_default_is_csharp9() {
        assert_eq!(CSharpVersion::default(), CSharpVersion::CSharp9);
    }

    #[test]
    fn version_ordering() {
        assert!(CSharpVersion::CSharp9 < CSharpVersion::CSharp10);
        assert!(CSharpVersion::CSharp10 < CSharpVersion::CSharp11);
        assert!(CSharpVersion::CSharp11 < CSharpVersion::CSharp12);
        assert!(CSharpVersion::CSharp12 >= CSharpVersion::CSharp10);
    }

    // --- CSharpNamespace tests ---

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

    // --- CSharpConfig tests ---

    #[test]
    fn default_config_has_expected_values() {
        let config = CSharpConfig::default();
        assert_eq!(config.namespace, "Generated");
        assert_eq!(config.serializer, Serializer::SystemTextJson);
        assert_eq!(config.target, CSharpVersion::CSharp9);
        assert_eq!(config.export_dir, PathBuf::from("./csharp-bindings"));
    }

    #[test]
    fn parses_full_metadata() {
        let toml = r#"
[package]
name = "test"
version = "0.1.0"

[package.metadata.csharp]
namespace = "PulsarAnvil.Types"
serializer = "newtonsoft"
target = "10.0"
export-dir = "generated/csharp"
"#;
        let config = CSharpConfig::from_toml_str(toml);
        assert_eq!(config.namespace, "PulsarAnvil.Types");
        assert_eq!(config.serializer, Serializer::Newtonsoft);
        assert_eq!(config.target, CSharpVersion::CSharp10);
        assert_eq!(config.export_dir, PathBuf::from("generated/csharp"));
    }

    #[test]
    fn missing_metadata_returns_defaults() {
        let toml = r#"
[package]
name = "test"
version = "0.1.0"
"#;
        let config = CSharpConfig::from_toml_str(toml);
        assert_eq!(config.namespace, "Generated");
        assert_eq!(config.serializer, Serializer::SystemTextJson);
    }

    #[test]
    fn partial_metadata_fills_defaults() {
        let toml = r#"
[package]
name = "test"
version = "0.1.0"

[package.metadata.csharp]
namespace = "MyApp"
"#;
        let config = CSharpConfig::from_toml_str(toml);
        assert_eq!(config.namespace, "MyApp");
        assert_eq!(config.serializer, Serializer::SystemTextJson);
        assert_eq!(config.target, CSharpVersion::CSharp9);
        assert_eq!(config.export_dir, PathBuf::from("./csharp-bindings"));
    }

    #[test]
    fn invalid_toml_returns_defaults() {
        let config = CSharpConfig::from_toml_str("not valid toml {{{{");
        assert_eq!(config.namespace, "Generated");
    }

    #[test]
    fn unknown_serializer_defaults_to_stj() {
        let toml = r#"
[package]
name = "test"
version = "0.1.0"

[package.metadata.csharp]
serializer = "unknown"
"#;
        let config = CSharpConfig::from_toml_str(toml);
        assert_eq!(config.serializer, Serializer::SystemTextJson);
    }

    #[test]
    fn unknown_target_defaults_to_csharp9() {
        let toml = r#"
[package]
name = "test"
version = "0.1.0"

[package.metadata.csharp]
target = "99.0"
"#;
        let config = CSharpConfig::from_toml_str(toml);
        assert_eq!(config.target, CSharpVersion::CSharp9);
    }

    #[test]
    fn invalid_namespace_in_toml_falls_back_to_default() {
        let toml = r#"
[package]
name = "test"
version = "0.1.0"

[package.metadata.csharp]
namespace = "123invalid"
"#;
        let config = CSharpConfig::from_toml_str(toml);
        assert_eq!(config.namespace, "Generated");
    }
}
