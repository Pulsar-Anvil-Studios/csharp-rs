// Rust guideline compliant 2026-02-14
//! Generate C# type definitions from Rust structs and enums.
//!
//! `csharp-rs` provides a derive macro that generates C# class, record,
//! or enum definitions from Rust types. It respects `serde` attributes
//! for JSON serialization compatibility, making it ideal for sharing
//! types between a Rust backend and a C#/.NET or Unity client.
//!
//! # Examples
//!
//! ```
//! use csharp_rs::CSharp;
//!
//! #[derive(CSharp)]
//! #[csharp(namespace = "Game.Types")]
//! pub struct PlayerProfile {
//!     pub name: String,
//!     pub level: i32,
//!     pub score: Option<f64>,
//! }
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Re-export of the derive macro from `csharp-rs-macros`.
#[doc(inline)]
pub use csharp_rs_macros::CSharp;

// ---------------------------------------------------------------------------
// Configuration enums
// ---------------------------------------------------------------------------

/// Which JSON serializer library to target in generated C# code.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Serializer {
    /// `System.Text.Json` attributes (default).
    #[default]
    SystemTextJson,
    /// `Newtonsoft.Json` attributes.
    Newtonsoft,
}

/// Target C# language version — controls which syntax features are used.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CSharpVersion {
    /// C# 9.0 (default) — positional records, block-scoped namespaces.
    #[default]
    CSharp9,
    /// C# 10.0 — file-scoped namespaces.
    CSharp10,
    /// C# 11.0 — `required` modifier, native `[JsonPolymorphic]`.
    CSharp11,
    /// C# 12.0 — primary constructors.
    CSharp12,
}

impl std::fmt::Display for CSharpVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::CSharp9 => "9.0",
            Self::CSharp10 => "10.0",
            Self::CSharp11 => "11.0",
            Self::CSharp12 => "12.0",
        };
        f.write_str(s)
    }
}

/// A validated C# namespace (e.g. `"Company.Product"`).
///
/// Each segment must start with an ASCII letter or underscore and contain
/// only ASCII alphanumeric characters or underscores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CSharpNamespace(String);

impl CSharpNamespace {
    /// Creates a new validated namespace.
    ///
    /// # Errors
    ///
    /// Returns an error message if the namespace is empty, contains empty
    /// segments, or has segments with invalid characters.
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let s = value.into();
        validate_namespace(&s)?;
        Ok(Self(s))
    }
}

impl std::fmt::Display for CSharpNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// Validates a C# namespace string.
fn validate_namespace(ns: &str) -> Result<(), &'static str> {
    if ns.is_empty() {
        return Err("namespace must not be empty");
    }
    for segment in ns.split('.') {
        if segment.is_empty() {
            return Err("namespace must not contain empty segments");
        }
        let mut chars = segment.chars();
        let first = chars.next().expect("segment is non-empty");
        if !first.is_ascii_alphabetic() && first != '_' {
            return Err("each segment must start with a letter or underscore");
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("segments must contain only letters, digits, or underscores");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Runtime configuration
// ---------------------------------------------------------------------------

/// Runtime configuration for C# code generation.
///
/// Controls namespace, serializer library, C# language version, and export
/// directory. Construct with [`Config::default`] and customize with builder
/// methods.
///
/// # Examples
///
/// ```
/// use csharp_rs::{Config, Serializer, CSharpVersion};
///
/// let cfg = Config::default()
///     .with_serializer(Serializer::Newtonsoft)
///     .with_target(CSharpVersion::CSharp11);
/// ```
#[derive(Debug)]
pub struct Config {
    namespace: CSharpNamespace,
    serializer: Serializer,
    target: CSharpVersion,
    export_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            namespace: CSharpNamespace::new("Generated").expect("default namespace is valid"),
            serializer: Serializer::SystemTextJson,
            target: CSharpVersion::CSharp9,
            export_dir: PathBuf::from("./csharp-bindings"),
        }
    }
}

impl Config {
    /// Sets the root namespace. Panics if the value is not a valid C#
    /// namespace.
    ///
    /// # Panics
    ///
    /// Panics if `ns` fails [`CSharpNamespace`] validation.
    #[must_use]
    pub fn with_namespace(mut self, ns: &str) -> Self {
        self.namespace =
            CSharpNamespace::new(ns).unwrap_or_else(|e| panic!("invalid namespace \"{ns}\": {e}"));
        self
    }

    /// Sets the root namespace from a pre-validated [`CSharpNamespace`].
    #[must_use]
    pub fn with_validated_namespace(mut self, ns: CSharpNamespace) -> Self {
        self.namespace = ns;
        self
    }

    /// Sets the target serializer library.
    #[must_use]
    pub fn with_serializer(mut self, serializer: Serializer) -> Self {
        self.serializer = serializer;
        self
    }

    /// Sets the target C# language version.
    #[must_use]
    pub fn with_target(mut self, target: CSharpVersion) -> Self {
        self.target = target;
        self
    }

    /// Sets the export directory for generated `.cs` files.
    #[must_use]
    pub fn with_export_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.export_dir = dir.into();
        self
    }

    /// Returns the configured namespace as a string slice.
    #[must_use]
    pub fn namespace(&self) -> &str {
        self.namespace.as_ref()
    }

    /// Returns the configured serializer.
    #[must_use]
    pub fn serializer(&self) -> Serializer {
        self.serializer
    }

    /// Returns the configured C# target version.
    #[must_use]
    pub fn target(&self) -> CSharpVersion {
        self.target
    }

    /// Returns the configured export directory.
    #[must_use]
    pub fn export_dir(&self) -> &Path {
        &self.export_dir
    }
}

/// Metadata for a C# field, used by `#[serde(flatten)]` to inline properties.
#[derive(Debug, Clone)]
pub enum CSharpFieldInfo {
    /// A regular property to inline into the parent record.
    Property {
        /// C# property name (`PascalCase`).
        property_name: String,
        /// JSON serialization key.
        json_name: String,
        /// Resolved C# type name (e.g. `"string"`, `"int"`).
        type_name: String,
        /// Whether the field is nullable.
        is_optional: bool,
    },
    /// An extension data container (from flattened `HashMap`).
    ExtensionData {
        /// C# key type name (typically `"string"`).
        key_type_name: String,
        /// C# value type name.
        value_type_name: String,
    },
}

/// Generates a C# type definition as a string.
///
/// Implementors produce a complete `.cs` file content including
/// `using` directives, namespace declaration, and type definition.
pub trait CSharp {
    /// Returns the C# type name (e.g., `"int"`, `"MyStruct"`).
    fn csharp_name(cfg: &Config) -> String;

    /// Returns the complete `.cs` file content for this type, or empty for
    /// primitives / generics.
    fn csharp_definition(cfg: &Config) -> String;

    /// Returns C# type names this type depends on (for transitive export).
    fn dependencies(cfg: &Config) -> Vec<String>;

    /// Returns metadata about this type's fields (used by `#[serde(flatten)]`).
    ///
    /// Only meaningful for struct types. Primitives, generics, and enums
    /// return an empty vec (the default implementation).
    #[must_use]
    fn csharp_fields(_cfg: &Config) -> Vec<CSharpFieldInfo> {
        Vec::new()
    }
}

/// Writes the C# definition of `T` to `path`.
///
/// Creates parent directories if they do not exist.
///
/// # Errors
///
/// Returns an I/O error if the file cannot be written.
pub fn export_to<T: CSharp>(cfg: &Config, path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, T::csharp_definition(cfg))
}

// ---------------------------------------------------------------------------
// Primitive type mappings
// ---------------------------------------------------------------------------

macro_rules! impl_csharp_primitive {
    ($rust_ty:ty, $csharp_name:expr) => {
        impl CSharp for $rust_ty {
            fn csharp_name(_cfg: &Config) -> String {
                String::from($csharp_name)
            }

            fn csharp_definition(_cfg: &Config) -> String {
                // Primitives have no standalone definition.
                String::new()
            }

            fn dependencies(_cfg: &Config) -> Vec<String> {
                Vec::new()
            }
        }
    };
}

impl_csharp_primitive!(String, "string");
impl_csharp_primitive!(bool, "bool");

// Signed integers
impl_csharp_primitive!(i8, "sbyte");
impl_csharp_primitive!(i16, "short");
impl_csharp_primitive!(i32, "int");
impl_csharp_primitive!(i64, "long");
// C# `decimal` (128-bit, 96-bit mantissa) cannot represent all `i128` values.
// `System.Int128` requires .NET 7+ / C# 11+, outside the default C# 9.0 target.
impl_csharp_primitive!(i128, "decimal");

// Unsigned integers
impl_csharp_primitive!(u8, "byte");
impl_csharp_primitive!(u16, "ushort");
impl_csharp_primitive!(u32, "uint");
impl_csharp_primitive!(u64, "ulong");
// C# `decimal` (128-bit, 96-bit mantissa) cannot represent all `u128` values.
// `System.UInt128` requires .NET 7+ / C# 11+, outside the default C# 9.0 target.
impl_csharp_primitive!(u128, "decimal");

// Floating point
impl_csharp_primitive!(f32, "float");
impl_csharp_primitive!(f64, "double");

// ---------------------------------------------------------------------------
// Generic type mappings
// ---------------------------------------------------------------------------

/// Returns the inner type name without a nullable suffix.
///
/// Nullability (`?`) is handled by the derive macro via the `is_optional`
/// flag in codegen, not by the trait. Calling `<Option<i32>>::csharp_name()`
/// returns `"int"`, not `"int?"`.
impl<T: CSharp> CSharp for Option<T> {
    fn csharp_name(cfg: &Config) -> String {
        T::csharp_name(cfg)
    }

    fn csharp_definition(_cfg: &Config) -> String {
        String::new()
    }

    fn dependencies(cfg: &Config) -> Vec<String> {
        vec![T::csharp_name(cfg)]
    }
}

impl<T: CSharp> CSharp for Vec<T> {
    fn csharp_name(cfg: &Config) -> String {
        format!("List<{}>", T::csharp_name(cfg))
    }

    fn csharp_definition(_cfg: &Config) -> String {
        String::new()
    }

    fn dependencies(cfg: &Config) -> Vec<String> {
        vec![T::csharp_name(cfg)]
    }
}

impl<K: CSharp, V: CSharp, S: std::hash::BuildHasher> CSharp for HashMap<K, V, S> {
    fn csharp_name(cfg: &Config) -> String {
        format!(
            "Dictionary<{}, {}>",
            K::csharp_name(cfg),
            V::csharp_name(cfg)
        )
    }

    fn csharp_definition(_cfg: &Config) -> String {
        String::new()
    }

    fn dependencies(cfg: &Config) -> Vec<String> {
        vec![K::csharp_name(cfg), V::csharp_name(cfg)]
    }
}

impl<T: CSharp, S: std::hash::BuildHasher> CSharp for HashSet<T, S> {
    fn csharp_name(cfg: &Config) -> String {
        format!("HashSet<{}>", T::csharp_name(cfg))
    }

    fn csharp_definition(_cfg: &Config) -> String {
        String::new()
    }

    fn dependencies(cfg: &Config) -> Vec<String> {
        vec![T::csharp_name(cfg)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializer_default_is_system_text_json() {
        assert_eq!(Serializer::default(), Serializer::SystemTextJson);
    }

    #[test]
    fn csharp_version_default_is_csharp9() {
        assert_eq!(CSharpVersion::default(), CSharpVersion::CSharp9);
    }

    #[test]
    fn csharp_version_ordering() {
        assert!(CSharpVersion::CSharp9 < CSharpVersion::CSharp10);
        assert!(CSharpVersion::CSharp10 < CSharpVersion::CSharp11);
        assert!(CSharpVersion::CSharp11 < CSharpVersion::CSharp12);
    }

    #[test]
    fn csharp_version_display() {
        assert_eq!(CSharpVersion::CSharp9.to_string(), "9.0");
        assert_eq!(CSharpVersion::CSharp10.to_string(), "10.0");
        assert_eq!(CSharpVersion::CSharp11.to_string(), "11.0");
        assert_eq!(CSharpVersion::CSharp12.to_string(), "12.0");
    }

    #[test]
    fn namespace_valid_single_segment() {
        let ns = CSharpNamespace::new("MyGame").unwrap();
        assert_eq!(ns.as_ref(), "MyGame");
    }

    #[test]
    fn namespace_valid_multi_segment() {
        let ns = CSharpNamespace::new("Company.Product.Module").unwrap();
        assert_eq!(ns.as_ref(), "Company.Product.Module");
    }

    #[test]
    fn namespace_underscore_prefix_valid() {
        assert!(CSharpNamespace::new("_Internal").is_ok());
    }

    #[test]
    fn namespace_invalid_empty() {
        assert!(CSharpNamespace::new("").is_err());
    }

    #[test]
    fn namespace_invalid_starts_with_digit() {
        assert!(CSharpNamespace::new("1Invalid").is_err());
    }

    #[test]
    fn namespace_invalid_special_chars() {
        assert!(CSharpNamespace::new("My-Namespace").is_err());
    }

    #[test]
    fn namespace_invalid_empty_segment() {
        assert!(CSharpNamespace::new("A..B").is_err());
    }

    #[test]
    fn namespace_display() {
        let ns = CSharpNamespace::new("Test.Ns").unwrap();
        assert_eq!(ns.to_string(), "Test.Ns");
    }

    #[test]
    fn namespace_partial_eq_str() {
        let ns = CSharpNamespace::new("Generated").unwrap();
        assert_eq!(ns, "Generated");
    }

    #[test]
    fn config_default_values() {
        let cfg = Config::default();
        assert_eq!(cfg.namespace(), "Generated");
        assert_eq!(cfg.serializer(), Serializer::SystemTextJson);
        assert_eq!(cfg.target(), CSharpVersion::CSharp9);
        assert_eq!(cfg.export_dir(), Path::new("./csharp-bindings"));
    }

    #[test]
    fn config_with_serializer() {
        let cfg = Config::default().with_serializer(Serializer::Newtonsoft);
        assert_eq!(cfg.serializer(), Serializer::Newtonsoft);
    }

    #[test]
    fn config_with_target() {
        let cfg = Config::default().with_target(CSharpVersion::CSharp12);
        assert_eq!(cfg.target(), CSharpVersion::CSharp12);
    }

    #[test]
    fn config_with_namespace() {
        let cfg = Config::default().with_namespace("My.Game");
        assert_eq!(cfg.namespace(), "My.Game");
    }

    #[test]
    #[should_panic(expected = "each segment must start with a letter")]
    fn config_with_namespace_panics_on_invalid() {
        let _ = Config::default().with_namespace("1Bad");
    }

    #[test]
    fn config_with_validated_namespace() {
        let ns = CSharpNamespace::new("Pre.Validated").unwrap();
        let cfg = Config::default().with_validated_namespace(ns);
        assert_eq!(cfg.namespace(), "Pre.Validated");
    }

    #[test]
    fn config_with_export_dir() {
        let cfg = Config::default().with_export_dir("./output");
        assert_eq!(cfg.export_dir(), Path::new("./output"));
    }

    #[test]
    fn config_builder_chaining() {
        let cfg = Config::default()
            .with_namespace("Unity.Types")
            .with_serializer(Serializer::Newtonsoft)
            .with_target(CSharpVersion::CSharp11)
            .with_export_dir("./generated");
        assert_eq!(cfg.namespace(), "Unity.Types");
        assert_eq!(cfg.serializer(), Serializer::Newtonsoft);
        assert_eq!(cfg.target(), CSharpVersion::CSharp11);
        assert_eq!(cfg.export_dir(), Path::new("./generated"));
    }

    #[test]
    fn string_maps_to_csharp_string() {
        let cfg = Config::default();
        assert_eq!(String::csharp_name(&cfg), "string");
    }

    #[test]
    fn bool_maps_to_csharp_bool() {
        let cfg = Config::default();
        assert_eq!(bool::csharp_name(&cfg), "bool");
    }

    #[test]
    fn integer_type_mappings() {
        let cfg = Config::default();
        assert_eq!(i8::csharp_name(&cfg), "sbyte");
        assert_eq!(i16::csharp_name(&cfg), "short");
        assert_eq!(i32::csharp_name(&cfg), "int");
        assert_eq!(i64::csharp_name(&cfg), "long");
        assert_eq!(i128::csharp_name(&cfg), "decimal");
        assert_eq!(u8::csharp_name(&cfg), "byte");
        assert_eq!(u16::csharp_name(&cfg), "ushort");
        assert_eq!(u32::csharp_name(&cfg), "uint");
        assert_eq!(u64::csharp_name(&cfg), "ulong");
        assert_eq!(u128::csharp_name(&cfg), "decimal");
    }

    #[test]
    fn float_type_mappings() {
        let cfg = Config::default();
        assert_eq!(f32::csharp_name(&cfg), "float");
        assert_eq!(f64::csharp_name(&cfg), "double");
    }

    #[test]
    fn option_unwraps_inner_type() {
        let cfg = Config::default();
        assert_eq!(<Option<i32>>::csharp_name(&cfg), "int");
    }

    #[test]
    fn vec_maps_to_list() {
        let cfg = Config::default();
        assert_eq!(<Vec<String>>::csharp_name(&cfg), "List<string>");
    }

    #[test]
    fn hashmap_maps_to_dictionary() {
        let cfg = Config::default();
        assert_eq!(
            <HashMap<String, i32>>::csharp_name(&cfg),
            "Dictionary<string, int>"
        );
    }

    #[test]
    fn hashset_maps_to_hashset() {
        let cfg = Config::default();
        assert_eq!(<HashSet<String>>::csharp_name(&cfg), "HashSet<string>");
    }

    #[test]
    fn nested_generics() {
        let cfg = Config::default();
        assert_eq!(<Vec<Option<i32>>>::csharp_name(&cfg), "List<int>");
        assert_eq!(
            <HashMap<String, Vec<f64>>>::csharp_name(&cfg),
            "Dictionary<string, List<double>>"
        );
    }

    // --- primitive csharp_definition / dependencies coverage ---

    #[test]
    fn primitive_definition_is_empty() {
        let cfg = Config::default();
        assert!(String::csharp_definition(&cfg).is_empty());
        assert!(bool::csharp_definition(&cfg).is_empty());
        assert!(i32::csharp_definition(&cfg).is_empty());
        assert!(u64::csharp_definition(&cfg).is_empty());
        assert!(f64::csharp_definition(&cfg).is_empty());
    }

    #[test]
    fn primitive_dependencies_is_empty() {
        let cfg = Config::default();
        assert!(String::dependencies(&cfg).is_empty());
        assert!(bool::dependencies(&cfg).is_empty());
        assert!(i32::dependencies(&cfg).is_empty());
        assert!(u64::dependencies(&cfg).is_empty());
        assert!(f64::dependencies(&cfg).is_empty());
    }

    // --- generic csharp_definition / dependencies coverage ---

    #[test]
    fn option_definition_is_empty() {
        let cfg = Config::default();
        assert!(<Option<i32>>::csharp_definition(&cfg).is_empty());
    }

    #[test]
    fn option_dependencies_contains_inner() {
        let cfg = Config::default();
        let deps = <Option<i32>>::dependencies(&cfg);
        assert_eq!(deps, vec!["int"]);
    }

    #[test]
    fn vec_definition_is_empty() {
        let cfg = Config::default();
        assert!(<Vec<String>>::csharp_definition(&cfg).is_empty());
    }

    #[test]
    fn vec_dependencies_contains_inner() {
        let cfg = Config::default();
        let deps = <Vec<String>>::dependencies(&cfg);
        assert_eq!(deps, vec!["string"]);
    }

    #[test]
    fn hashmap_definition_is_empty() {
        let cfg = Config::default();
        assert!(<HashMap<String, i32>>::csharp_definition(&cfg).is_empty());
    }

    #[test]
    fn hashmap_dependencies_contains_key_and_value() {
        let cfg = Config::default();
        let deps = <HashMap<String, i32>>::dependencies(&cfg);
        assert_eq!(deps, vec!["string", "int"]);
    }

    #[test]
    fn hashset_definition_is_empty() {
        let cfg = Config::default();
        assert!(<HashSet<String>>::csharp_definition(&cfg).is_empty());
    }

    #[test]
    fn hashset_dependencies_contains_inner() {
        let cfg = Config::default();
        let deps = <HashSet<String>>::dependencies(&cfg);
        assert_eq!(deps, vec!["string"]);
    }

    // --- csharp_fields coverage ---

    #[test]
    fn primitive_csharp_fields_is_empty() {
        let cfg = Config::default();
        assert!(String::csharp_fields(&cfg).is_empty());
        assert!(i32::csharp_fields(&cfg).is_empty());
        assert!(bool::csharp_fields(&cfg).is_empty());
    }

    #[test]
    fn generic_csharp_fields_is_empty() {
        let cfg = Config::default();
        assert!(<Vec<String>>::csharp_fields(&cfg).is_empty());
        assert!(<Option<i32>>::csharp_fields(&cfg).is_empty());
        assert!(<HashMap<String, i32>>::csharp_fields(&cfg).is_empty());
        assert!(<HashSet<String>>::csharp_fields(&cfg).is_empty());
    }

    // --- export_to coverage ---

    #[test]
    fn export_to_writes_file() {
        let cfg = Config::default();
        let dir = std::env::temp_dir().join("csharp_rs_test_export");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join("Test.cs");

        export_to::<i32>(&cfg, &path).expect("export_to should succeed");

        let content = std::fs::read_to_string(&path).expect("file should exist");
        // Primitives have empty definitions
        assert!(content.is_empty());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
