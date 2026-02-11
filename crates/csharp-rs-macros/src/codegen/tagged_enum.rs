// Rust guideline compliant 2026-02-11
//! Tagged enum code generation (internally, adjacently, externally tagged,
//! and untagged).

use crate::config::CSharpConfig;
use crate::types::{EnumTagging, TaggedVariant};
use proc_macro2::TokenStream;

/// Builds a tagged enum definition token stream.
///
/// Dispatches to the appropriate code generation strategy based on the
/// [`EnumTagging`] variant (internal, adjacent, external, or untagged).
///
/// # Panics
///
/// Currently unimplemented; panics with a `todo!` message.
pub fn build_tagged_enum_definition(
    _csharp_name: &str,
    _namespace: &str,
    _tagging: &EnumTagging,
    _variants: &[TaggedVariant],
    _config: &CSharpConfig,
) -> TokenStream {
    todo!("tagged enum codegen \u{2014} Task 5")
}
