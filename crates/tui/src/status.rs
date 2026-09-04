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
use bravebot_i18n::t;
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
    /// What the server reported using on the last turn, or `None` before one has run.
    ///
    /// Observed rather than configured, which is the whole point: the endpoint answers a model name
    /// it will not serve by substituting a weaker one, so what was asked for does not establish what
    /// answered.
    pub served_model: Option<&'a str>,
    /// Whether the last turn actually spent a subscription credential.
    ///
    /// `None` before any turn has run. Three states rather than a bool because "not yet known" is
    /// not the same as "no", and reporting a guess as an observation is the bug this replaced.
    pub premium: Option<bool>,
    pub theme: &'a str,
    pub config: &'a Config,
    pub confinement: &'a str,
    pub turns: usize,
    pub tokens: u64,
    /// Where the session's wall clock went, every turn added together.
    ///
    /// Beside the token count because it is the other half of what a session cost. A person
    /// reading this wants to know which of the three things to do something about, and only the
    /// split can tell them: a faster model, a faster test suite, or fewer prompts.
    pub timing: bravebot_agent::timing::Timing,
    pub trust: &'a TrustStore,
    pub programs: &'a TrustedPrograms,
}

/// Compose the report.
pub fn report(facts: &Facts<'_>) -> Report {
    let mut lines = Vec::new();

    lines.push(Line::new(
        t!(status_session),
        if facts.session_name.is_empty() {
            t!(status_session_untitled).to_string()
        } else {
            facts.session_name.to_string()
        },
    ));
    lines.push(Line::new(t!(status_session_id), facts.session_id));

    let trusted = facts.trust.is_trusted(".");
    lines.push(
        Line::new(t!(status_directory), abbreviate(facts.directory)).with_note(if trusted {
            t!(status_directory_trusted)
        } else {
            t!(status_directory_untrusted)
        }),
    );

    for added in facts.added_directories {
        lines.push(
            Line::new(t!(status_also_open), abbreviate(added))
                .with_note(t!(status_added_directory)),
        );
    }

    lines.push(match facts.model {
        Some(model) => Line::new(t!(status_model), model).with_note(t!(status_model_chosen)),
        None => Line::new(t!(status_model), &facts.config.default_model)
            .with_note(t!(status_model_default)),
    });

    // What actually answered, where that is not what was asked for. The endpoint substitutes a
    // model it will not serve rather than refusing, so the line above can name Opus for a whole
    // session that was answered by something else every turn. Reported beside it, since the two
    // together are the fact and either alone is misleading.
    if let Some(served) = facts.served_model.filter(|served| {
        let asked = facts.model.unwrap_or(&facts.config.default_model);
        // `automatic` is the server's choice by definition, so a concrete name coming back is the
        // feature working rather than a substitution worth flagging.
        asked != *served && asked != bravebot_config::DEFAULT_MODEL
    }) {
        lines.push(Line::new(t!(status_served), served).with_note(t!(status_served_instead)));
    }

    lines.push(Line::new(t!(status_theme), facts.theme).with_note(t!(status_theme_chosen)));

    // The environment rather than the host. See the note at the top of this file.
    //
    // The note says which tier the last turn actually ran on, not whether this build knows a premium
    // host. It used to say the latter, which is baked in at compile time and true of every build:
    // a session whose subscription was never read still reported "premium configured" while every
    // request went out on the free tier and came back answered by a weaker model. What a person
    // wants from this line is which tier they are getting, and that is a fact about a request.
    lines.push(
        Line::new(t!(status_endpoint), environment(&facts.config.endpoint)).with_note(
            match (facts.config.premium_endpoint.is_some(), facts.premium) {
                (true, Some(true)) => t!(status_premium_in_use),
                (true, Some(false)) => t!(status_premium_not_spent),
                // Nothing has run yet, so nothing has been observed. Saying which tier is in use
                // before a request has been made would be the same guess as before.
                (_, None) | (false, _) => configured_tier(facts.config),
            },
        ),
    );

    lines.push(Line::new(t!(status_confinement), facts.confinement));

    lines.push(Line::new(
        t!(status_this_session),
        format!(
            "{} · {}",
            t!(count_turns, count = facts.turns),
            tokens(facts.tokens)
        ),
    ));

    // Under the turn and token counts, because it is the same question about the same session:
    // what did this cost. Drawn only once a turn has run, since every figure would be zero before
    // that and a panel of zeroes reads as a broken feature rather than as an idle session.
    //
    // The threshold is a whole second rather than any time at all, because the figures are rendered
    // by the same formatter the indicator uses and it floors to seconds: a part of 400ms would be
    // drawn as `0s`, which reads as "none" beside a note saying where the time went.
    if facts.timing.wall_ms >= 1_000 {
        lines.push(Line::new(
            t!(status_time),
            crate::indicator::format_elapsed(std::time::Duration::from_millis(
                facts.timing.wall_ms,
            )),
        ));
        // Only the parts that happened. A session that never ran a tool has nothing to say about
        // tool time, and a zero beside it invites the reader to work out whether it means "none" or
        // "not measured".
        for (millis, note) in [
            (facts.timing.inference_ms, t!(status_time_inference)),
            (facts.timing.tools_ms, t!(status_time_tools)),
            (facts.timing.stalled_ms, t!(status_time_stalled)),
            (facts.timing.overhead_ms(), t!(status_time_overhead)),
        ] {
            if millis >= 1_000 {
                lines.push(
                    Line::new(
                        "",
                        crate::indicator::format_elapsed(std::time::Duration::from_millis(millis)),
                    )
                    .with_note(note),
                );
            }
        }
    }

    // Last because it is the part that grows. What a write recorded is the thing nothing else
    // reports: a file an earlier turn marked untrusted is invisible until it refuses to be read.
    let rules: Vec<(&str, Integrity)> = facts.trust.rules().collect();
    if rules.is_empty() {
        lines.push(Line::new(t!(status_trust), t!(status_nothing_vouched_for)));
    } else {
        lines.push(Line::new(
            t!(status_trust),
            t!(count_rules, count = rules.len()),
        ));
        for (path, integrity) in rules.iter() {
            let shown = if path.is_empty() { "." } else { path };
            lines.push(match integrity {
                Integrity::Trusted => Line::new("", shown).with_note(t!(status_trusted)),
                Integrity::Untrusted => Line::new("", shown).with_note(t!(status_untrusted)),
            });
        }
    }

    // A standing permission the user gave earlier and cannot otherwise see. Every other prompt in
    // this session announces itself by appearing; this is the one that stops appearing, so without
    // a line here there is nothing to tell them a command now runs unasked and that what it prints
    // is being read as trusted.
    let vouched: Vec<&bravebot_core::programs::Command> = facts.programs.iter().collect();
    if vouched.is_empty() {
        lines.push(Line::new(
            t!(status_programs),
            t!(status_every_run_is_asked),
        ));
    } else {
        lines.push(
            Line::new(
                t!(status_trusted_commands),
                t!(count_commands, count = vouched.len()),
            )
            .with_note(t!(status_trusted_commands_note)),
        );
        for command in vouched.iter().take(MAX_COMMANDS) {
            lines.push(Line::new("", command.display()));
        }
        if vouched.len() > MAX_COMMANDS {
            lines.push(Line::new(
                "",
                t!(status_and_more, count = vouched.len() - MAX_COMMANDS),
            ));
        }
    }

    Report { lines }
}

/// What can be said about the tier before a request has settled it.
///
/// The configuration and nothing else, which is why the premium wording stops short of claiming a
/// credential will be spent. A stored batch may still be expired, exhausted, or issued for another
/// environment, and which of those holds is settled by a request rather than by looking. Reading the
/// file at startup would license a firmer claim than the file supports.
///
/// Shared with the opening screen rather than written twice, so the line drawn at startup and the
/// line `/status` shows an hour later cannot drift apart.
pub fn configured_tier(config: &Config) -> &'static str {
    match config.premium_endpoint {
        Some(_) => t!(status_premium_available),
        None => t!(status_free_tier),
    }
}

/// Which deployment an endpoint names, without naming the host.
///
/// Matched on the host rather than parsed, because the answer wanted is one of three words and a
/// URL parser would still leave the mapping to be written. An unrecognised host is reported as
/// custom rather than guessed at.
fn environment(endpoint: &str) -> &'static str {
    if endpoint.contains("127.0.0.1") || endpoint.contains("localhost") {
        t!(environment_local)
    } else if endpoint.contains(".brave.software") {
        t!(environment_dev)
    } else if endpoint.contains(".brave.com") {
        t!(environment_prod)
    } else {
        t!(environment_custom)
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

/// Tokens, in the units a person reads them in.
fn tokens(count: u64) -> String {
    if count < 1_000 {
        return t!(count_tokens, count = count);
    }
    // The one fraction the interface shows, so the one place a language that does not write a
    // point between a whole number and its fraction has anything to say about it.
    let thousands =
        format!("{:.1}", count as f64 / 1_000.0).replace('.', t!(number_decimal_separator));
    t!(count_tokens_thousands, thousands = thousands)
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
            // Nothing observed, which is what a session looks like before its first turn. Tests
            // about the tier and the served model set these themselves.
            served_model: None,
            premium: None,
            theme: "brave",
            config,
            confinement: "kernel-enforced",
            turns: 4,
            tokens: 12_400,
            // Nothing measured, which is what a session looks like before its first turn. Tests
            // about the time report set this themselves.
            timing: bravebot_agent::timing::Timing::default(),
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

    /// The bug this replaced. Every build knows a premium host, so reporting premium from the
    /// configuration said "premium" for a session whose credentials were never read, while every
    /// request went out on the free tier and came back answered by a weaker model. A status panel
    /// that cannot be trusted on this point is worse than one that omits it.
    #[test]
    fn the_tier_reported_is_the_one_the_last_turn_actually_ran_on() {
        let config = config_for(
            "https://ai-chat.bsg.brave.com",
            Some("https://ai-chat-premium.bsg.brave.com"),
        );
        let trust = trusting();

        // A premium host is configured and a turn ran without spending anything. The old line said
        // "premium configured" here, which is the sentence that hid a whole broken session.
        let mut free = facts(&config, &trust);
        free.premium = Some(false);
        let shown = rendered(&report(&free));
        assert!(shown.contains("no subscription was used"), "{shown}");
        assert!(
            !shown.contains("premium, a credential"),
            "a free-tier turn was reported as premium: {shown}"
        );

        let mut premium = facts(&config, &trust);
        premium.premium = Some(true);
        let shown = rendered(&report(&premium));
        assert!(shown.contains("a credential was spent"), "{shown}");
    }

    /// What the opening screen draws before anything has run is what `/status` says at that moment,
    /// because both take it from here. Two copies of this wording would be two things to keep true,
    /// and the one that drifted would be the one nobody re-reads.
    ///
    /// Neither reads the store. A batch on disk may be expired, exhausted, or for the wrong
    /// environment, so its presence is not the tier: only a request settles that.
    #[test]
    fn the_opening_line_and_the_panel_say_the_same_thing_before_a_turn_runs() {
        let premium = config_for(
            "https://ai-chat.bsg.brave.com",
            Some("https://ai-chat-premium.bsg.brave.com"),
        );
        let trust = trusting();
        let shown = rendered(&report(&facts(&premium, &trust)));
        assert!(shown.contains(configured_tier(&premium)), "{shown}");

        // And a build that cannot reach premium at all says so in both places.
        let free = config_for("https://ai-chat.bsg.brave.com", None);
        assert_eq!(configured_tier(&free), t!(status_free_tier));
        let shown = rendered(&report(&facts(&free, &trust)));
        assert!(shown.contains(configured_tier(&free)), "{shown}");
    }

    /// Before the first turn nothing has been observed, so the panel says premium is available
    /// rather than claiming it is or is not in use. Claiming either would be the same guess the
    /// configuration line used to make.
    #[test]
    fn a_session_with_no_turn_yet_does_not_claim_a_tier() {
        let config = config_for(
            "https://ai-chat.bsg.brave.com",
            Some("https://ai-chat-premium.bsg.brave.com"),
        );
        let trust = trusting();
        let shown = rendered(&report(&facts(&config, &trust)));
        assert!(shown.contains("nothing sent yet"), "{shown}");
    }

    /// The endpoint answers a model name it will not serve by substituting a weaker one, with a 200
    /// and an ordinary reply. So a panel that reports only what was asked for names a model that
    /// never answered anything.
    #[test]
    fn a_substituted_model_is_reported_beside_the_one_asked_for() {
        let config = config_for("https://ai-chat.bsg.brave.com", None);
        let trust = trusting();

        let mut substituted = facts(&config, &trust);
        substituted.model = Some("claude-opus");
        substituted.served_model = Some("qwen-14b-instruct");
        let shown = rendered(&report(&substituted));
        // Both halves: what was chosen, and what actually answered.
        assert!(shown.contains("claude-opus"), "{shown}");
        assert!(shown.contains("qwen-14b-instruct"), "{shown}");
        assert!(shown.contains("served instead"), "{shown}");

        // Served what was asked for: nothing to report, or the line would be on every session.
        let mut honoured = facts(&config, &trust);
        honoured.model = Some("claude-opus");
        honoured.served_model = Some("claude-opus");
        let shown = rendered(&report(&honoured));
        assert!(!shown.contains("served instead"), "{shown}");
    }

    /// `automatic` is the server choosing per request, so a concrete name coming back is the feature
    /// working rather than a substitution. Flagging it would put a warning on the default config.
    #[test]
    fn automatic_being_resolved_to_a_real_model_is_not_a_substitution() {
        let config = config_for("https://ai-chat.bsg.brave.com", None);
        let trust = trusting();
        let mut automatic = facts(&config, &trust);
        automatic.model = None;
        automatic.served_model = Some("claude-3-haiku");

        let shown = rendered(&report(&automatic));
        assert!(!shown.contains("served instead"), "{shown}");
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

    /// A total is unactionable. The panel has to say which of the three things took the time, since
    /// the answer decides whether a person wants a faster model, a faster test suite, or fewer
    /// prompts.
    #[test]
    fn the_panel_says_where_the_session_spent_its_time() {
        let config = config_for("http://127.0.0.1:1", None);
        let trust = trusting();
        let mut spent = facts(&config, &trust);
        spent.timing = bravebot_agent::timing::Timing {
            wall_ms: 600_000,
            inference_ms: 120_000,
            tools_ms: 60_000,
            stalled_ms: 400_000,
        };

        let shown = rendered(&report(&spent));
        assert!(
            shown.contains("10m 00s"),
            "the total was not reported: {shown}"
        );
        assert!(shown.contains("waiting on you"), "{shown}");
        // The remainder is the figure with nobody to blame for it, and it is the one nothing else
        // can show.
        assert!(shown.contains("unaccounted for"), "{shown}");
        assert!(
            shown.contains("6m 40s"),
            "the stall was not reported: {shown}"
        );
    }

    /// Before a turn has run every figure is zero, and a panel of zeroes reads as a broken feature
    /// rather than as a session that has not started.
    #[test]
    fn a_session_with_no_turn_yet_reports_no_time() {
        let config = config_for("http://127.0.0.1:1", None);
        let trust = trusting();
        let shown = rendered(&report(&facts(&config, &trust)));
        assert!(!shown.contains("waiting on you"), "{shown}");
        assert!(!shown.contains("unaccounted for"), "{shown}");
    }

    /// A part that did not happen says nothing rather than `0s`, which beside a note about where the
    /// time went reads as a measurement rather than as an absence.
    #[test]
    fn a_part_that_never_happened_is_not_reported_as_zero() {
        let config = config_for("http://127.0.0.1:1", None);
        let trust = trusting();
        let mut quiet = facts(&config, &trust);
        // A session that was never asked anything and ran no tool: all of it on the model.
        quiet.timing = bravebot_agent::timing::Timing {
            wall_ms: 30_000,
            inference_ms: 30_000,
            tools_ms: 0,
            stalled_ms: 0,
        };

        let shown = rendered(&report(&quiet));
        assert!(shown.contains("on the model"), "{shown}");
        assert!(
            !shown.contains("waiting on you"),
            "a stall that never happened was reported: {shown}"
        );
        assert!(!shown.contains("running tools"), "{shown}");
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
        assert_eq!(t!(count_turns, count = 1), "1 turn");
        assert_eq!(t!(count_turns, count = 0), "0 turns");
        assert_eq!(t!(count_rules, count = 4), "4 rules");
        assert_eq!(tokens(940), "940 tokens");
        assert_eq!(tokens(1), "1 token");
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
