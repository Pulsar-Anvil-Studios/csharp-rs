// Rust guideline compliant 2026-03-14
//! `CSharp` implementations for `chrono` date and time types.
//!
//! Enabled by the `chrono-impl` feature.

use crate::{Config, CSharp};

impl_csharp_primitive!(chrono::NaiveDate, "DateOnly");
impl_csharp_primitive!(chrono::NaiveTime, "TimeOnly");
impl_csharp_primitive!(chrono::NaiveDateTime, "DateTime");
impl_csharp_primitive!(chrono::Duration, "TimeSpan");

/// `DateTime<Tz>` maps to `DateTimeOffset` regardless of timezone.
impl<Tz: chrono::TimeZone> CSharp for chrono::DateTime<Tz> {
    fn csharp_name(_cfg: &Config) -> String {
        String::from("DateTimeOffset")
    }

    fn csharp_definition(_cfg: &Config) -> String {
        String::new()
    }

    fn dependencies(_cfg: &Config) -> Vec<String> {
        Vec::new()
    }
}
