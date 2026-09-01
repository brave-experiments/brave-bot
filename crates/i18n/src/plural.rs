// Which plural category a number falls into, per language.
//
// The build script includes this file directly, so it must stay dependency-free: it decides at
// build time whether a catalog's language has a rule here, and the generated code calls it at
// run time to pick a variant.
//
// These are CLDR's categories, and only the rule families actually written out below. A language
// with no rule here is a build error rather than a default, because defaulting means a
// translation ships reading correctly in the singular and wrongly everywhere else, and nobody
// finds that from the English side.

/// The rule families spelled out, and which languages use each.
///
/// Grouped by rule rather than listed per language so that adding a language is picking an
/// existing family or writing a new one, never copying a body and editing a number.
enum Family {
    /// `one` for exactly 1. English, German, Dutch, the Nordics, Italian, Spanish, Greek.
    OneIsSingular,
    /// `one` for 0 and 1. French and Brazilian Portuguese count zero as singular.
    ZeroAndOneAreSingular,
    /// No count-driven distinction at all: the noun does not change.
    NoDistinction,
}

/// Language subtag to rule family.
///
/// Region is deliberately not consulted: `pt-BR` and `pt-PT` disagree about zero, so `pt` is
/// absent here rather than guessed at, and a catalog for either has to say which it wants by
/// adding the entry.
fn family(language: &str) -> Option<Family> {
    match language {
        "en" | "de" | "nl" | "sv" | "da" | "nb" | "nn" | "it" | "es" | "el" | "fi" | "et" => {
            Some(Family::OneIsSingular)
        }
        "fr" => Some(Family::ZeroAndOneAreSingular),
        "ja" | "ko" | "zh" | "vi" | "th" | "id" | "ms" => Some(Family::NoDistinction),
        _ => None,
    }
}

/// Whether a catalog in this language can have its selects compiled.
///
/// Called by the build script, so a translation whose plurals nothing here knows how to form
/// fails the build that would have shipped it.
pub fn is_known(language: &str) -> bool {
    family(language).is_some()
}

/// The category `count` falls into, as the catalog spells variant keys.
///
/// An unknown language answers `other` rather than panicking: the build already refused to
/// generate a select for one, so reaching this means a caller passed something no catalog
/// produced, and a running agent should not abort over a plural.
pub fn category(language: &str, count: i64) -> &'static str {
    match family(language) {
        Some(Family::OneIsSingular) => {
            if count == 1 {
                "one"
            } else {
                "other"
            }
        }
        Some(Family::ZeroAndOneAreSingular) => {
            if count == 0 || count == 1 {
                "one"
            } else {
                "other"
            }
        }
        Some(Family::NoDistinction) | None => "other",
    }
}
