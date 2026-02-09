// Rust guideline compliant 2026-02-09
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
//! #[csharp(export, namespace = "Game.Types")]
//! pub struct PlayerProfile {
//!     pub name: String,
//!     pub level: i32,
//!     pub score: Option<f64>,
//! }
//! ```

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
