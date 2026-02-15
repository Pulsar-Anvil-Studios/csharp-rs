// Rust guideline compliant 2026-02-10
//! Proc macro implementation for the `csharp-rs` crate.
//!
//! This crate provides the `#[derive(CSharp)]` macro that generates C# type
//! definitions from Rust structs and enums. It is not intended to be used
//! directly; use `csharp-rs` instead, which re-exports the derive macro.

mod attr;
mod codegen;
mod config;
mod types;

use config::CSharpConfig;
use proc_macro::TokenStream;

/// Derives a C# type definition for the annotated Rust type.
///
/// Supports `#[csharp(...)]` attributes for customization and respects
/// `#[serde(...)]` attributes for JSON serialization compatibility.
///
/// # Examples
///
/// ```ignore
/// use csharp_rs::CSharp;
///
/// #[derive(CSharp)]
/// #[csharp(export)]
/// struct Player {
///     name: String,
///     score: i32,
/// }
/// ```
#[proc_macro_derive(CSharp, attributes(csharp, serde))]
pub fn derive_csharp(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| String::from("."));
    let config = CSharpConfig::from_manifest_dir(std::path::Path::new(&manifest_dir));

    match types::process_input(&input) {
        Ok(derived) => derived.into_token_stream(&config).into(),
        Err(err) => err.to_compile_error().into(),
    }
}
