// Rust guideline compliant 2026-02-10
//! Attribute parsing for `#[serde(...)]` and `#[csharp(...)]`.
//!
//! Provides the [`Inflection`] enum for case-conversion conventions
//! and sub-modules for container-level and field-level attribute parsing.

pub mod container;
pub mod field;

/// Naming convention applied by `serde(rename_all = "...")`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inflection {
    /// `lowercase`
    Lower,
    /// `UPPERCASE`
    Upper,
    /// `PascalCase`
    Pascal,
    /// `camelCase`
    Camel,
    /// `snake_case`
    Snake,
    /// `SCREAMING_SNAKE_CASE`
    ScreamingSnake,
    /// `kebab-case`
    Kebab,
    /// `SCREAMING-KEBAB-CASE`
    ScreamingKebab,
}

impl Inflection {
    /// Parses a serde `rename_all` string into an [`Inflection`].
    ///
    /// # Errors
    ///
    /// Returns `None` if the string is not a recognized convention.
    pub fn from_rename_all(s: &str) -> Option<Self> {
        match s {
            "lowercase" => Some(Self::Lower),
            "UPPERCASE" => Some(Self::Upper),
            "PascalCase" => Some(Self::Pascal),
            "camelCase" => Some(Self::Camel),
            "snake_case" => Some(Self::Snake),
            "SCREAMING_SNAKE_CASE" => Some(Self::ScreamingSnake),
            "kebab-case" => Some(Self::Kebab),
            "SCREAMING-KEBAB-CASE" => Some(Self::ScreamingKebab),
            _ => None,
        }
    }

    /// Applies this inflection convention to a Rust `snake_case` identifier.
    #[must_use]
    pub fn apply(self, name: &str) -> String {
        let words = split_words(name);
        match self {
            Self::Lower => words.join(""),
            Self::Upper => {
                let upper: Vec<String> = words.iter().map(|w| w.to_uppercase()).collect();
                upper.join("")
            }
            Self::Pascal => words.iter().map(|w| capitalize(w)).collect(),
            Self::Camel => {
                let mut result = String::new();
                for (i, w) in words.iter().enumerate() {
                    if i == 0 {
                        result.push_str(&w.to_lowercase());
                    } else {
                        result.push_str(&capitalize(w));
                    }
                }
                result
            }
            Self::Snake => words.join("_"),
            Self::ScreamingSnake => {
                let upper: Vec<String> = words.iter().map(|w| w.to_uppercase()).collect();
                upper.join("_")
            }
            Self::Kebab => words.join("-"),
            Self::ScreamingKebab => {
                let upper: Vec<String> = words.iter().map(|w| w.to_uppercase()).collect();
                upper.join("-")
            }
        }
    }
}

/// Splits a `snake_case` identifier into lowercase words.
fn split_words(name: &str) -> Vec<String> {
    name.split('_')
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Capitalizes the first character of a word.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => {
            let upper: String = c.to_uppercase().collect();
            upper + &chars.as_str().to_lowercase()
        }
    }
}

/// Converts a Rust `snake_case` name to `PascalCase`.
#[must_use]
pub fn to_pascal_case(name: &str) -> String {
    Inflection::Pascal.apply(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflection_from_rename_all() {
        assert_eq!(
            Inflection::from_rename_all("camelCase"),
            Some(Inflection::Camel)
        );
        assert_eq!(
            Inflection::from_rename_all("PascalCase"),
            Some(Inflection::Pascal)
        );
        assert_eq!(
            Inflection::from_rename_all("snake_case"),
            Some(Inflection::Snake)
        );
        assert_eq!(
            Inflection::from_rename_all("SCREAMING_SNAKE_CASE"),
            Some(Inflection::ScreamingSnake)
        );
        assert_eq!(
            Inflection::from_rename_all("kebab-case"),
            Some(Inflection::Kebab)
        );
        assert_eq!(
            Inflection::from_rename_all("SCREAMING-KEBAB-CASE"),
            Some(Inflection::ScreamingKebab)
        );
        assert_eq!(
            Inflection::from_rename_all("lowercase"),
            Some(Inflection::Lower)
        );
        assert_eq!(
            Inflection::from_rename_all("UPPERCASE"),
            Some(Inflection::Upper)
        );
        assert_eq!(Inflection::from_rename_all("invalid"), None);
    }

    #[test]
    fn apply_camel_case() {
        assert_eq!(Inflection::Camel.apply("player_id"), "playerId");
        assert_eq!(Inflection::Camel.apply("score"), "score");
        assert_eq!(
            Inflection::Camel.apply("max_health_points"),
            "maxHealthPoints"
        );
    }

    #[test]
    fn apply_pascal_case() {
        assert_eq!(Inflection::Pascal.apply("player_id"), "PlayerId");
        assert_eq!(Inflection::Pascal.apply("score"), "Score");
    }

    #[test]
    fn apply_snake_case() {
        assert_eq!(Inflection::Snake.apply("player_id"), "player_id");
    }

    #[test]
    fn apply_screaming_snake() {
        assert_eq!(Inflection::ScreamingSnake.apply("player_id"), "PLAYER_ID");
    }

    #[test]
    fn apply_kebab_case() {
        assert_eq!(Inflection::Kebab.apply("player_id"), "player-id");
    }

    #[test]
    fn apply_screaming_kebab() {
        assert_eq!(Inflection::ScreamingKebab.apply("player_id"), "PLAYER-ID");
    }

    #[test]
    fn apply_lowercase() {
        assert_eq!(Inflection::Lower.apply("player_id"), "playerid");
    }

    #[test]
    fn apply_uppercase() {
        assert_eq!(Inflection::Upper.apply("player_id"), "PLAYERID");
    }

    #[test]
    fn to_pascal_case_works() {
        assert_eq!(to_pascal_case("player_profile"), "PlayerProfile");
        assert_eq!(to_pascal_case("id"), "Id");
    }
}
