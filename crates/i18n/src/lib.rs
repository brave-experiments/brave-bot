//! Every word this program says to a person, and the machinery for saying it in their language.
//!
//! Catalogs live in `locales/`, one file per locale, in a subset of Project Fluent. The build
//! script compiles them into Rust, so what ships is `&'static str`s and the code that assembles
//! them, and a message is reached through [`t!`]:
//!
//! ```ignore
//! use bravebot_i18n::t;
//!
//! block.title(t!(confirm_write_title));
//! footer.push(t!(status_turns, count = session.turns));
//! ```
//!
//! ## What is not in here
//!
//! Only text a person reads. The words the planner reads are not translated and do not belong in
//! a catalog: a tool's description, the preamble, and the `refused: …` a tool answers with are all
//! part of the interface to the model, and rewording them in another language changes what the
//! model does. They are pinned by the specs for exactly that reason. The rule is the audience, not
//! the crate: a string in `bravebot-agent` that ends up on the screen is a message, and a string
//! in `bravebot-tui` that ends up in a request is not.
//!
//! ## Why a key can never be a value
//!
//! [`t!`] takes the message name as a bare word, and the macro has one arm per message in the
//! reference catalog. There is no arm that accepts a runtime string, so there is no way to write
//! a lookup whose result depends on a value, which means no path by which content the agent was
//! handed can choose what is said. A misspelled name does not fall back and does not render a key
//! on somebody's screen: it fails to compile, naming the line.
//!
//! Argument names are checked the same way, by being the fields of a generated struct. Passing an
//! argument the message does not take, or forgetting one it does, is a compilation error.
//!
//! ```
//! assert_eq!(bravebot_i18n::t!(count_turns, count = 2), "2 turns");
//! ```
//!
//! A name no catalog defines:
//!
//! ```compile_fail
//! bravebot_i18n::t!(no_message_is_called_this);
//! ```
//!
//! An argument the message does not take:
//!
//! ```compile_fail
//! bravebot_i18n::t!(count_turns, quantity = 2);
//! ```
//!
//! A key that is a value rather than a name, which is the shape this crate exists to make
//! unwritable:
//!
//! ```compile_fail
//! let chosen = untrusted_file_contents();
//! bravebot_i18n::t!(chosen);
//! ```

/// The catalog format and its parser, shared with the build script that reads it.
pub mod catalog;
/// Which plural category a number falls into, per language.
pub mod plural;

/// The compiled catalogs: one item per message, plus [`Locale`] and the [`t!`] arms.
pub mod messages {
    include!(concat!(env!("OUT_DIR"), "/messages.rs"));
}

pub use messages::{DEFAULT, LOCALES, Locale};

use std::sync::OnceLock;

/// Set to a locale tag to override what the environment says, mostly so a test can pin one.
pub const LOCALE: &str = "BRAVEBOT_LOCALE";

/// A number a message counts on.
///
/// A distinct type rather than an integer so that every width a call site might hold converts on
/// the way in, and so a plural select can be sure it has something to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Count(i64);

impl Count {
    pub(crate) fn get(self) -> i64 {
        self.0
    }
}

impl std::fmt::Display for Count {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

macro_rules! counts {
    (lossless $($from:ty)*) => { $(
        impl From<$from> for Count {
            fn from(value: $from) -> Self {
                Count(i64::from(value))
            }
        }
    )* };
    (saturating $($from:ty)*) => { $(
        impl From<$from> for Count {
            // A count past i64 is a count no message is going to read out anyway, and an
            // interface that panicked while drawing a number would be worse than one off by
            // the last few quintillion.
            fn from(value: $from) -> Self {
                Count(i64::try_from(value).unwrap_or(i64::MAX))
            }
        }
    )* };
}

counts!(lossless u8 u16 u32 i8 i16 i32 i64);
counts!(saturating u64 usize i128 u128 isize);

static CHOSEN: OnceLock<Locale> = OnceLock::new();

/// The catalog this run is using.
///
/// The reference until something says otherwise, which for the shipped binaries is one call to
/// [`init_from_environment`] at startup. A library that read the environment the first time
/// anything drew a word would make every test's output depend on the machine it ran on, and the
/// point of a rendered-output test is that two people reading it see the same thing.
pub fn locale() -> Locale {
    *CHOSEN.get().unwrap_or(&DEFAULT)
}

/// Fix the catalog for the rest of the process. The first call wins.
///
/// A person does not change their language partway through a session, and an interface that
/// redrew half of itself in another one would be worse than one that asked them to start again.
pub fn init(locale: Locale) {
    let _ = CHOSEN.set(locale);
}

/// Fix the catalog from what the user's shell says their language is.
///
/// `BRAVEBOT_LOCALE` first so a person can have one program in a language their whole shell is
/// not, then the POSIX variables in the precedence POSIX gives them.
pub fn init_from_environment() {
    let requested = [LOCALE, "LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()));
    init(match requested {
        Some(requested) => resolve(&requested),
        None => DEFAULT,
    });
}

/// Which catalog answers a request.
pub fn resolve(requested: &str) -> Locale {
    match best(
        requested,
        &LOCALES.iter().map(|l| l.tag()).collect::<Vec<_>>(),
    ) {
        Some(index) => LOCALES[index],
        None => DEFAULT,
    }
}

/// Which of `available` best answers `requested`, by the usual widening: the exact locale, then
/// any locale in the same language, then nothing.
///
/// Written against tags rather than against the catalogs that shipped so that the rule can be
/// tested on locale sets this build does not have.
fn best(requested: &str, available: &[&str]) -> Option<usize> {
    // POSIX spells "no locale at all" as C, and the encoding and modifier a shell appends say
    // nothing about which language the words should be in.
    let tag = requested
        .trim()
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .replace('_', "-");
    if tag.is_empty() || tag.eq_ignore_ascii_case("c") || tag.eq_ignore_ascii_case("posix") {
        return None;
    }

    if let Some(exact) = available
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(&tag))
    {
        return Some(exact);
    }

    let language = tag.split('-').next().unwrap_or_default();
    available.iter().position(|candidate| {
        candidate
            .split('-')
            .next()
            .is_some_and(|other| other.eq_ignore_ascii_case(language))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIPPED: [&str; 4] = ["en-US", "fr-CA", "fr-FR", "ja"];

    /// A rendered-output test has to say the same thing on every machine that runs it, so nothing
    /// about the environment reaches a message unless a binary asked for it.
    #[test]
    fn a_process_that_never_chose_a_locale_reads_the_reference() {
        assert_eq!(locale(), DEFAULT);
        assert_eq!(DEFAULT.tag(), "en-US");
    }

    /// The whole point of naming a catalog after a locale.
    #[test]
    fn a_request_takes_the_catalog_of_that_exact_name() {
        assert_eq!(best("fr-FR", &SHIPPED), Some(2));
    }

    /// A French speaker in Belgium is better served by any French than by English.
    #[test]
    fn a_request_with_no_catalog_of_its_own_takes_one_in_its_language() {
        assert_eq!(best("fr-BE", &SHIPPED), Some(1));
    }

    /// Nothing here says which region, so the first catalog in the language answers.
    #[test]
    fn a_bare_language_takes_a_catalog_in_that_language() {
        assert_eq!(best("fr", &SHIPPED), Some(1));
    }

    /// A locale with no catalog at all is what the fallback exists for.
    #[test]
    fn a_language_that_did_not_ship_falls_back() {
        assert_eq!(best("de-DE", &SHIPPED), None);
        assert_eq!(resolve("de-DE"), DEFAULT);
    }

    /// What a shell actually exports, which is neither a bare tag nor spelled with a hyphen.
    #[test]
    fn the_encoding_and_modifier_a_shell_appends_are_not_part_of_the_tag() {
        assert_eq!(best("fr_FR.UTF-8", &SHIPPED), Some(2));
        assert_eq!(best("fr_FR.UTF-8@euro", &SHIPPED), Some(2));
    }

    /// `C` is POSIX for "do not localise anything", not a request for a language called C.
    #[test]
    fn the_posix_locale_asks_for_no_catalog() {
        assert_eq!(best("C", &SHIPPED), None);
        assert_eq!(best("POSIX", &SHIPPED), None);
        assert_eq!(best("", &SHIPPED), None);
    }

    /// Counts come from lengths, from indices, and from parsed numbers, and no call site should
    /// have to cast one on the way to a message.
    #[test]
    fn a_count_is_made_from_whatever_width_the_caller_holds() {
        let lines: Vec<u8> = vec![1, 2, 3];
        assert_eq!(t!(count_turns, count = lines.len()), "3 turns");
        assert_eq!(t!(count_turns, count = 1_u32), "1 turn");
        assert_eq!(t!(count_turns, count = -4_i64), "-4 turns");
    }

    /// A count too large to be a count is still not worth aborting the interface over.
    #[test]
    fn a_count_past_what_a_message_could_read_saturates_rather_than_panicking() {
        assert_eq!(Count::from(u64::MAX), Count::from(i64::MAX));
    }

    /// The reason the catalog has selects at all.
    #[test]
    fn a_plural_select_picks_the_variant_the_language_calls_for() {
        assert_eq!(t!(count_turns, count = 0), "0 turns");
        assert_eq!(t!(count_turns, count = 1), "1 turn");
        assert_eq!(t!(count_turns, count = 2), "2 turns");
    }

    /// Every language whose plural rule is written down has to be one a catalog may be named for.
    #[test]
    fn a_language_with_a_plural_rule_is_the_only_kind_a_select_may_ship_in() {
        assert!(plural::is_known("en"));
        assert!(!plural::is_known("qq"));
        assert_eq!(plural::category("en", 1), "one");
        assert_eq!(plural::category("en", 0), "other");
        assert_eq!(plural::category("fr", 0), "one");
        assert_eq!(plural::category("ja", 1), "other");
    }
}
