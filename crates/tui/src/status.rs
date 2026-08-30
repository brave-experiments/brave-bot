//! What `/status` reports.
//!
//! Built as lines rather than printed, so what it says can be tested without a terminal and shown
//! in the transcript like any other note.
//!
//! # What it deliberately leaves out
//!
//! Not the endpoint host and not the key id, though `bravebot doctor` prints both. A status panel is the
//! thing people paste into an issue or a screenshot, and an internal hostname is the part worth not
//! spreading. Which environment is in use answers the question people actually have, which is
//! whether they are pointed at dev or prod.
//!
//! Nothing here is labelled content. The trust rules are the user's own decisions, the paths in them
//! are workspace-relative names shown to the person who owns the workspace, and the counts are this
//! program's own arithmetic. No model reads any of it.

use bravebot_config::Config;
use bravebot_core::label::Integrity;
use bravebot_core::programs::TrustedPrograms;
use bravebot_core::trust::TrustStore;
use std::path::Path;

/// How many vouched commands are listed before the rest become a count.
///
/// The trust map is listed in full because a rule nobody can read is a file whose footing has to
/// be remembered. This list is capped for now; whether that is the same problem is #57.
const MAX_COMMANDS: usize = 6;

/// One line of the report: a label, a value, and an optional aside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub label: String,
    pub value: String,
    /// Why the value is what it is, where that is not obvious.
    pub note: String,
}

impl Line {
    fn new(label: &str, value: impl Into<String>) -> Self {
        Self {
            label: label.to_string(),
            value: value.into(),
            note: String::new(),
        }
    }

    fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }
}

/// Everything `/status` has to say, in the order it says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub lines: Vec<Line>,
}

/// What the session knows about itself, passed in rather than read here.
///
/// A struct rather than eight arguments, and borrowed rather than owned, because this only reads.
pub struct Facts<'a> {
    pub session_name: &'a str,
    pub session_id: &'a str,
    pub directory: &'a Path,
    pub added_directories: &'a [std::path::PathBuf],
    pub model: Option<&'a str>,
    pub config: &'a Config,
    pub confinement: &'a str,
    pub turns: usize,
    pub tokens: u64,
    pub trust: &'a TrustStore,
    pub programs: &'a TrustedPrograms,
}

/// Compose the report.
pub fn report(facts: &Facts<'_>) -> Report {
    let mut lines = Vec::new();

    lines.push(Line::new(
        "Session",
        if facts.session_name.is_empty() {
            "untitled, nothing sent yet".to_string()
        } else {
            facts.session_name.to_string()
        },
    ));
    lines.push(Line::new("Session id", facts.session_id));

    let trusted = facts.trust.is_trusted(".");
    lines.push(
        Line::new("Directory", abbreviate(facts.directory)).with_note(if trusted {
            "trusted"
        } else {
            "not trusted, so every write is shown to you"
        }),
    );

    for added in facts.added_directories {
        lines.push(Line::new("Also open", abbreviate(added)).with_note("added with /add-dir"));
    }

    lines.push(match facts.model {
        Some(model) => Line::new("Model", model).with_note("chosen with /model"),
        None => Line::new("Model", &facts.config.default_model).with_note("the configured default"),
    });

    // The environment rather than the host. See the note at the top of this file.
    lines.push(
        Line::new("Endpoint", environment(&facts.config.endpoint)).with_note(
            match facts.config.premium_endpoint {
                Some(_) => "premium configured",
                None => "free tier only",
            },
        ),
    );

    lines.push(Line::new("Confinement", facts.confinement));

    lines.push(Line::new(
        "This session",
        format!("{} · {}", plural(facts.turns, "turn"), tokens(facts.tokens)),
    ));

    // Last because it is the part that grows. What a write recorded is the thing nothing else
    // reports: a file an earlier turn marked untrusted is invisible until it refuses to be read.
    let rules: Vec<(&str, Integrity)> = facts.trust.rules().collect();
    if rules.is_empty() {
        lines.push(Line::new("Trust", "nothing vouched for"));
    } else {
        lines.push(Line::new("Trust", plural(rules.len(), "rule")));
        for (path, integrity) in rules.iter() {
            let shown = if path.is_empty() { "." } else { path };
            lines.push(match integrity {
                Integrity::Trusted => Line::new("", shown).with_note("trusted"),
                Integrity::Untrusted => Line::new("", shown).with_note("untrusted"),
            });
        }
    }

    // A standing permission the user gave earlier and cannot otherwise see. Every other prompt in
    // this session announces itself by appearing; this is the one that stops appearing, so without
    // a line here there is nothing to tell them a command now runs unasked and that what it prints
    // is being read as trusted.
    let vouched: Vec<&bravebot_core::programs::Command> = facts.programs.iter().collect();
    if vouched.is_empty() {
        lines.push(Line::new("Programs", "every run is put to you"));
    } else {
        lines.push(
            Line::new("Trusted commands", plural(vouched.len(), "command"))
                .with_note("run unasked, and their output is trusted"),
        );
        for command in vouched.iter().take(MAX_COMMANDS) {
            lines.push(Line::new("", command.display()));
        }
        if vouched.len() > MAX_COMMANDS {
            lines.push(Line::new(
                "",
                format!("… and {} more", vouched.len() - MAX_COMMANDS),
            ));
        }
    }

    Report { lines }
}

/// Which deployment an endpoint names, without naming the host.
///
/// Matched on the host rather than parsed, because the answer wanted is one of three words and a
/// URL parser would still leave the mapping to be written. An unrecognised host is reported as
/// custom rather than guessed at.
fn environment(endpoint: &str) -> &'static str {
    if endpoint.contains("127.0.0.1") || endpoint.contains("localhost") {
        "local"
    } else if endpoint.contains(".brave.software") {
        "dev"
    } else if endpoint.contains(".brave.com") {
        "prod"
    } else {
        "custom"
    }
}

/// A path with the home directory written as `~`, which is shorter and less personal.
fn abbreviate(path: &Path) -> String {
    let shown = path.display().to_string();
    let Some(home) = std::env::var_os("HOME") else {
        return shown;
    };
    let home = Path::new(&home).display().to_string();
    if home.is_empty() {
        return shown;
    }
    match shown.strip_prefix(&home) {
        Some(rest) => format!("~{rest}"),
        None => shown,
    }
}

/// "1 turn", "4 turns".
fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Tokens, in the units a person reads them in.
fn tokens(count: u64) -> String {
    if count < 1_000 {
        format!("{count} tokens")
    } else {
        format!("{:.1}k tokens", count as f64 / 1_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_for(endpoint: &str, premium: Option<&str>) -> Config {
        Config::from_lookup(|key| match key {
            "SERVICES_KEY_AICHAT" => Some("a-signing-key".into()),
            "BRAVE_SERVICES_KEY_ID" => Some("a-key-id".into()),
            "BRAVE_AI_CHAT_ENDPOINT" => Some(endpoint.to_string()),
            "BRAVE_AI_CHAT_PREMIUM_ENDPOINT" => premium.map(str::to_string),
            _ => None,
        })
        .expect("config")
    }

    /// Leaked once so a `Facts` built by the helper can borrow it for the test's lifetime.
    static NOTHING_VOUCHED: std::sync::LazyLock<TrustedPrograms> =
        std::sync::LazyLock::new(TrustedPrograms::new);

    fn trusting() -> TrustStore {
        let mut trust = TrustStore::new();
        trust.trust(".");
        trust
    }

    /// Facts with nothing vouched for, which is what a session that has not been asked looks like.
    /// Tests about the programs line build their own list and set it.
    fn facts<'a>(config: &'a Config, trust: &'a TrustStore) -> Facts<'a> {
        Facts {
            session_name: "the parser bug",
            session_id: "1787860306-65099",
            directory: Path::new("/tmp/project"),
            added_directories: &[],
            model: None,
            config,
            confinement: "kernel-enforced",
            turns: 4,
            tokens: 12_400,
            trust,
            programs: &NOTHING_VOUCHED,
        }
    }

    /// The one standing permission that stops announcing itself. Every other prompt in a session
    /// is visible by appearing; this is the one that makes prompts stop, so without a line here a
    /// user has no way to find out that a program now runs unasked.
    #[test]
    fn the_report_names_the_programs_that_run_without_asking() {
        let config = config_for("http://127.0.0.1:1", None);
        let trust = trusting();
        let vouched = TrustedPrograms::from_iter([
            bravebot_core::programs::Command::new("/usr/bin/git", vec!["log".to_string()]),
            bravebot_core::programs::Command::new("/usr/bin/make", vec!["check".to_string()]),
        ]);
        let mut facts = facts(&config, &trust);
        facts.programs = &vouched;

        let shown = rendered(&report(&facts));
        // The arguments are part of what was vouched for, so they are part of what is reported:
        // "git" alone would not tell a reader which command they trusted.
        assert!(shown.contains("/usr/bin/git log"), "{shown}");
        assert!(shown.contains("/usr/bin/make check"), "{shown}");
        // Both halves of the grant, said where the user can see them.
        assert!(shown.contains("output is trusted"), "{shown}");
    }

    /// The ordinary case has to say so rather than say nothing, or a user reading the report
    /// cannot tell the difference between "no program is vouched for" and "this report does not
    /// cover programs".
    #[test]
    fn a_session_that_vouched_for_nothing_says_every_run_is_asked_about() {
        let config = config_for("http://127.0.0.1:1", None);
        let trust = trusting();
        let shown = rendered(&report(&facts(&config, &trust)));
        assert!(shown.contains("every run is put to you"), "{shown}");
    }

    fn rendered(report: &Report) -> String {
        report
            .lines
            .iter()
            .map(|line| format!("{} {} {}\n", line.label, line.value, line.note))
            .collect()
    }

    /// The endpoint host and the key id are what a screenshot should not spread, and `doctor` is
    /// where they belong. This is the one property worth pinning hardest.
    #[test]
    fn the_host_and_the_key_id_are_never_reported() {
        let config = config_for(
            "https://ai-chat.bsg.brave.software",
            Some("https://ai-chat-premium.bsg.brave.software"),
        );
        let trust = trusting();
        let shown = rendered(&report(&facts(&config, &trust)));

        assert!(!shown.contains("ai-chat"), "the host was reported: {shown}");
        assert!(!shown.contains("a-key-id"), "the key id was reported");
        assert!(!shown.contains("a-signing-key"), "the key was reported");
        assert!(shown.contains("dev"), "the environment was not reported");
    }

    #[test]
    fn each_environment_is_named_without_its_host() {
        assert_eq!(environment("https://ai-chat.bsg.brave.software"), "dev");
        assert_eq!(environment("https://ai-chat.bsg.brave.com"), "prod");
        assert_eq!(environment("http://127.0.0.1:8080"), "local");
        assert_eq!(environment("https://example.invalid"), "custom");
    }

    /// Whether this directory is trusted is the first thing a person wants from a status panel, and
    /// a declined directory has to say what that means rather than only that it happened.
    #[test]
    fn the_directory_says_whether_it_is_trusted() {
        let config = config_for("http://127.0.0.1:1", None);

        let trusted = trusting();
        let shown = rendered(&report(&facts(&config, &trusted)));
        assert!(shown.contains("trusted"), "{shown}");

        let declined = TrustStore::new();
        let shown = rendered(&report(&facts(&config, &declined)));
        assert!(shown.contains("not trusted"), "{shown}");
        assert!(shown.contains("every write is shown"), "{shown}");
    }

    /// A chosen model and the configured default are different facts, and reporting one as the other
    /// would explain the wrong thing.
    #[test]
    fn the_model_says_whether_it_was_chosen() {
        let config = config_for("http://127.0.0.1:1", None);
        let trust = trusting();

        let shown = rendered(&report(&facts(&config, &trust)));
        assert!(shown.contains("automatic"), "{shown}");
        assert!(shown.contains("the configured default"), "{shown}");

        let mut chosen = facts(&config, &trust);
        chosen.model = Some("claude-3-sonnet");
        let shown = rendered(&report(&chosen));
        assert!(shown.contains("claude-3-sonnet"), "{shown}");
        assert!(shown.contains("chosen with /model"), "{shown}");
    }

    /// The markings a write recorded are the part nothing else reports: a poisoned file is otherwise
    /// invisible until something refuses to read it.
    #[test]
    fn an_untrusted_path_a_write_recorded_is_reported() {
        let config = config_for("http://127.0.0.1:1", None);
        let mut trust = trusting();
        trust.distrust("vendor/lib.js");

        let shown = rendered(&report(&facts(&config, &trust)));
        assert!(shown.contains("vendor/lib.js"), "{shown}");
        assert!(shown.contains("untrusted"), "{shown}");
    }

    /// Every rule is readable back however many there are. A session that wrote a dozen files
    /// holds a rule each, and a rule the panel will not show is one whose subject has to be
    /// remembered instead, which is the thing this report exists to save anyone doing.
    #[test]
    fn every_trust_rule_is_listed_however_many_there_are() {
        let config = config_for("http://127.0.0.1:1", None);
        let mut trust = trusting();
        for index in 0..12 {
            trust.distrust(&format!("file{index}.txt"));
        }

        let shown = rendered(&report(&facts(&config, &trust)));
        for index in 0..12 {
            assert!(shown.contains(&format!("file{index}.txt")), "{shown}");
        }
    }

    /// A directory opened with /add-dir is reachable and vouched for, so a panel that omitted it
    /// would understate what the session can touch.
    #[test]
    fn an_added_directory_is_reported() {
        let config = config_for("http://127.0.0.1:1", None);
        let trust = trusting();
        let added = vec![std::path::PathBuf::from("/tmp/notes")];
        let mut with_added = facts(&config, &trust);
        with_added.added_directories = &added;

        let shown = rendered(&report(&with_added));
        assert!(shown.contains("/tmp/notes"), "{shown}");
        assert!(shown.contains("added with /add-dir"), "{shown}");
    }

    /// A session with nothing sent has no name yet, and saying so is better than an empty line.
    #[test]
    fn a_session_with_no_name_says_so() {
        let config = config_for("http://127.0.0.1:1", None);
        let trust = trusting();
        let mut fresh = facts(&config, &trust);
        fresh.session_name = "";

        let shown = rendered(&report(&fresh));
        assert!(shown.contains("nothing sent yet"), "{shown}");
    }

    #[test]
    fn counts_read_the_way_a_person_says_them() {
        assert_eq!(plural(1, "turn"), "1 turn");
        assert_eq!(plural(0, "turn"), "0 turns");
        assert_eq!(plural(4, "rule"), "4 rules");
        assert_eq!(tokens(940), "940 tokens");
        assert_eq!(tokens(12_400), "12.4k tokens");
    }

    /// The home directory is both longer and more personal than `~`, and a status panel is pasted
    /// into issues.
    #[test]
    fn a_path_under_home_is_abbreviated() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let home = Path::new(&home);
        assert_eq!(
            abbreviate(&home.join("projects/bravebot")),
            "~/projects/bravebot"
        );
        assert_eq!(abbreviate(Path::new("/tmp/elsewhere")), "/tmp/elsewhere");
    }
}
