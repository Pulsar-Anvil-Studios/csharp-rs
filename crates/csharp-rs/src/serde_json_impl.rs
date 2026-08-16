//! `CSharp` implementations for `serde_json` types.
//!
//! Enabled by the `serde-json-impl` feature.
//!
//! `serde_json::Value` maps to a config-dependent C# type:
//! - **System.Text.Json**: `JsonElement`
//! - **Newtonsoft.Json**: `JToken`

use crate::{CSharp, Config, Serializer};

impl_csharp_primitive!(serde_json::Number, "double");

/// `serde_json::Value` maps to `JsonElement` (STJ) or `JToken` (Newtonsoft).
impl CSharp for serde_json::Value {
    fn csharp_name(cfg: &Config) -> String {
        match cfg.serializer() {
            Serializer::SystemTextJson => String::from("JsonElement"),
            Serializer::Newtonsoft => String::from("JToken"),
        }
    }

    fn csharp_definition(_cfg: &Config) -> String {
        String::new()
    }

    fn dependencies(_cfg: &Config) -> Vec<String> {
        Vec::new()
    }
}
