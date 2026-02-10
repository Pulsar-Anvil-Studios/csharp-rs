// Rust guideline compliant 2026-02-10
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
use std::path::Path;

/// Re-export of the derive macro from `csharp-rs-macros`.
#[doc(inline)]
pub use csharp_rs_macros::CSharp;

/// Generates a C# type definition as a string.
///
/// Implementors produce a complete `.cs` file content including
/// `using` directives, namespace declaration, and type definition.
pub trait CSharp {
    /// Returns the C# type name (e.g., `"PlayerProfile"`).
    fn csharp_name() -> String;

    /// Returns the complete C# type definition as file content.
    fn csharp_definition() -> String;

    /// Returns type names that this definition depends on.
    fn dependencies() -> Vec<String>;
}

/// Writes the C# definition of `T` to `path`.
///
/// Creates parent directories if they do not exist.
///
/// # Errors
///
/// Returns an I/O error if the file cannot be written.
pub fn export_to<T: CSharp>(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, T::csharp_definition())
}

// ---------------------------------------------------------------------------
// Primitive type mappings
// ---------------------------------------------------------------------------

macro_rules! impl_csharp_primitive {
    ($rust_ty:ty, $csharp_name:expr) => {
        impl CSharp for $rust_ty {
            fn csharp_name() -> String {
                String::from($csharp_name)
            }

            fn csharp_definition() -> String {
                // Primitives have no standalone definition.
                String::new()
            }

            fn dependencies() -> Vec<String> {
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
    fn csharp_name() -> String {
        T::csharp_name()
    }

    fn csharp_definition() -> String {
        String::new()
    }

    fn dependencies() -> Vec<String> {
        vec![T::csharp_name()]
    }
}

impl<T: CSharp> CSharp for Vec<T> {
    fn csharp_name() -> String {
        format!("List<{}>", T::csharp_name())
    }

    fn csharp_definition() -> String {
        String::new()
    }

    fn dependencies() -> Vec<String> {
        vec![T::csharp_name()]
    }
}

impl<K: CSharp, V: CSharp, S: std::hash::BuildHasher> CSharp for HashMap<K, V, S> {
    fn csharp_name() -> String {
        format!("Dictionary<{}, {}>", K::csharp_name(), V::csharp_name())
    }

    fn csharp_definition() -> String {
        String::new()
    }

    fn dependencies() -> Vec<String> {
        vec![K::csharp_name(), V::csharp_name()]
    }
}

impl<T: CSharp, S: std::hash::BuildHasher> CSharp for HashSet<T, S> {
    fn csharp_name() -> String {
        format!("HashSet<{}>", T::csharp_name())
    }

    fn csharp_definition() -> String {
        String::new()
    }

    fn dependencies() -> Vec<String> {
        vec![T::csharp_name()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_maps_to_csharp_string() {
        assert_eq!(String::csharp_name(), "string");
    }

    #[test]
    fn bool_maps_to_csharp_bool() {
        assert_eq!(bool::csharp_name(), "bool");
    }

    #[test]
    fn integer_type_mappings() {
        assert_eq!(i8::csharp_name(), "sbyte");
        assert_eq!(i16::csharp_name(), "short");
        assert_eq!(i32::csharp_name(), "int");
        assert_eq!(i64::csharp_name(), "long");
        assert_eq!(i128::csharp_name(), "decimal");
        assert_eq!(u8::csharp_name(), "byte");
        assert_eq!(u16::csharp_name(), "ushort");
        assert_eq!(u32::csharp_name(), "uint");
        assert_eq!(u64::csharp_name(), "ulong");
        assert_eq!(u128::csharp_name(), "decimal");
    }

    #[test]
    fn float_type_mappings() {
        assert_eq!(f32::csharp_name(), "float");
        assert_eq!(f64::csharp_name(), "double");
    }

    #[test]
    fn option_unwraps_inner_type() {
        assert_eq!(<Option<i32>>::csharp_name(), "int");
    }

    #[test]
    fn vec_maps_to_list() {
        assert_eq!(<Vec<String>>::csharp_name(), "List<string>");
    }

    #[test]
    fn hashmap_maps_to_dictionary() {
        assert_eq!(
            <HashMap<String, i32>>::csharp_name(),
            "Dictionary<string, int>"
        );
    }

    #[test]
    fn hashset_maps_to_hashset() {
        assert_eq!(<HashSet<String>>::csharp_name(), "HashSet<string>");
    }

    #[test]
    fn nested_generics() {
        assert_eq!(
            <Vec<Option<i32>>>::csharp_name(),
            "List<int>"
        );
        assert_eq!(
            <HashMap<String, Vec<f64>>>::csharp_name(),
            "Dictionary<string, List<double>>"
        );
    }

    // --- primitive csharp_definition / dependencies coverage ---

    #[test]
    fn primitive_definition_is_empty() {
        assert!(String::csharp_definition().is_empty());
        assert!(bool::csharp_definition().is_empty());
        assert!(i32::csharp_definition().is_empty());
        assert!(u64::csharp_definition().is_empty());
        assert!(f64::csharp_definition().is_empty());
    }

    #[test]
    fn primitive_dependencies_is_empty() {
        assert!(String::dependencies().is_empty());
        assert!(bool::dependencies().is_empty());
        assert!(i32::dependencies().is_empty());
        assert!(u64::dependencies().is_empty());
        assert!(f64::dependencies().is_empty());
    }

    // --- generic csharp_definition / dependencies coverage ---

    #[test]
    fn option_definition_is_empty() {
        assert!(<Option<i32>>::csharp_definition().is_empty());
    }

    #[test]
    fn option_dependencies_contains_inner() {
        let deps = <Option<i32>>::dependencies();
        assert_eq!(deps, vec!["int"]);
    }

    #[test]
    fn vec_definition_is_empty() {
        assert!(<Vec<String>>::csharp_definition().is_empty());
    }

    #[test]
    fn vec_dependencies_contains_inner() {
        let deps = <Vec<String>>::dependencies();
        assert_eq!(deps, vec!["string"]);
    }

    #[test]
    fn hashmap_definition_is_empty() {
        assert!(<HashMap<String, i32>>::csharp_definition().is_empty());
    }

    #[test]
    fn hashmap_dependencies_contains_key_and_value() {
        let deps = <HashMap<String, i32>>::dependencies();
        assert_eq!(deps, vec!["string", "int"]);
    }

    #[test]
    fn hashset_definition_is_empty() {
        assert!(<HashSet<String>>::csharp_definition().is_empty());
    }

    #[test]
    fn hashset_dependencies_contains_inner() {
        let deps = <HashSet<String>>::dependencies();
        assert_eq!(deps, vec!["string"]);
    }

    // --- export_to coverage ---

    #[test]
    fn export_to_writes_file() {
        let dir = std::env::temp_dir().join("csharp_rs_test_export");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join("Test.cs");

        export_to::<i32>(&path).expect("export_to should succeed");

        let content = std::fs::read_to_string(&path).expect("file should exist");
        // Primitives have empty definitions
        assert!(content.is_empty());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
