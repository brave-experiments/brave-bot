//! An incognito session adds nothing to `~/.bravebot`.
//!
//! These run against a real directory, redirected by `HOME`, because the claim is about the
//! filesystem afterwards rather than about which branch some function took. Every test here asks
//! the same question in a different place: after doing the thing that ordinarily writes, is there
//! anything on disk that was not there before?
//!
//! # Why a binary of its own
//!
//! Engaging is a one-way door for the life of a process, which is the property that makes it
//! trustworthy and also the reason these cannot share a binary with [`persist`]. An integration
//! test is its own process, so this file engages once at the top of each test and no other test
//! in the workspace is affected.
//!
//! # Why each test also reads
//!
//! A test asserting that nothing was written passes just as well when nothing was attempted: a
//! mistyped `HOME`, a helper that silently did nothing. So each test below first reads back
//! something it seeded, which fails unless the directory really is the one the code under test is
//! looking at. The read is the proof that the absence of a write means something.
//!
//! [`persist`]: ../persist.rs

use bravebot_tui::history::Entry;
use bravebot_tui::sessions::{Handle, Standing};
use bravebot_tui::store;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// Serialises the tests in this binary, since `HOME` is process-wide.
static HOME: Mutex<()> = Mutex::new(());

/// A scratch home holding whatever a test seeded, so a test never touches the real one.
struct Scratch {
    home: PathBuf,
    project: PathBuf,
    /// Dropped last, releasing `HOME` only once this test's directory is gone.
    _lock: MutexGuard<'static, ()>,
}

impl Scratch {
    /// A scratch `HOME` with the mode already engaged.
    ///
    /// Engaged here rather than in each test so no test can forget, which would turn it into one
    /// that asserts the ordinary behaviour and passes for the wrong reason.
    fn incognito(name: &str) -> Self {
        let lock = HOME.lock().unwrap_or_else(|held| held.into_inner());
        let root = std::env::temp_dir().join(format!("bravebot-incognito-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
        // SAFETY: `HOME` is held for as long as this value lives, so no other test in this binary
        // is reading or writing the variable while this one owns it.
        unsafe { std::env::set_var("HOME", &home) };
        bravebot_core::incognito::engage();
        Self {
            home,
            project,
            _lock: lock,
        }
    }

    /// The directory the mode is about.
    fn own_directory(&self) -> PathBuf {
        self.home.join(".bravebot")
    }

    /// Put a file in the user's own directory, as a previous ordinary session would have left it.
    fn seed(&self, name: &str, contents: &str) {
        let dir = self.own_directory();
        std::fs::create_dir_all(&dir).expect("seed the directory");
        std::fs::write(dir.join(name), contents).expect("seed a file");
    }

    /// Everything inside the user's own directory now, relative to it and sorted.
    ///
    /// A listing rather than a check of one path, so a write landing somewhere unexpected is
    /// caught by the same assertion as one landing where it was expected.
    fn contents(&self) -> Vec<String> {
        let root = self.own_directory();
        let mut found = Vec::new();
        walk(&root, &root, &mut found);
        found.sort();
        found
    }
}

/// Every file under `dir`, named relative to `root`.
fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, found);
        } else if let Ok(relative) = path.strip_prefix(root) {
            found.push(relative.display().to_string());
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.home.parent().expect("a root"));
    }
}

/// A prompt sent now from nowhere in particular.
fn sent(prompt: &str) -> Entry {
    Entry::sent(prompt, None)
}

/// INCOG-1: a prompt typed in an incognito session is not written down.
#[test]
fn no_prompt_is_written_down() {
    let scratch = Scratch::incognito("history");
    scratch.seed("history", "1700000000\t\tan earlier prompt\n");

    // The read that proves the directory below is the one being written to.
    assert_eq!(
        store::load_history()
            .iter()
            .map(|entry| entry.prompt.clone())
            .collect::<Vec<_>>(),
        vec!["an earlier prompt".to_string()],
        "the seeded history was not read back, so this test proves nothing"
    );

    store::append_history(&sent("something private"));
    store::save_history(&[sent("something private"), sent("and another")]);

    assert_eq!(
        std::fs::read_to_string(scratch.own_directory().join("history")).expect("the seeded file"),
        "1700000000\t\tan earlier prompt\n",
        "an incognito session added a prompt to the history file"
    );
    assert_eq!(scratch.contents(), vec!["history".to_string()]);
}

/// INCOG-2: choices made in an incognito session apply to it and outlive nothing.
#[test]
fn a_choice_applies_to_the_session_and_is_not_recorded() {
    let scratch = Scratch::incognito("choices");
    scratch.seed("model", "claude-sonnet-4-5\n");
    scratch.seed("theme", "nord\n");

    // Reading is the half that must keep working: a private session still opens with the model and
    // theme the person chose, which is what makes it a private session rather than a fresh install.
    assert_eq!(store::load_model().as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(store::load_theme().as_deref(), Some("nord"));

    store::save_model("some-other-model");
    store::save_theme("brave");
    store::save_effort(Some(bravebot_aichat::protocol::Effort::Xhigh));
    store::save_editing(bravebot_tui::vim::Editing::Vi);

    assert_eq!(
        store::load_model().as_deref(),
        Some("claude-sonnet-4-5"),
        "an incognito session overwrote the recorded model"
    );
    assert_eq!(
        store::load_theme().as_deref(),
        Some("nord"),
        "an incognito session overwrote the recorded theme"
    );
    assert_eq!(
        store::load_effort(),
        None,
        "an incognito session recorded an effort level"
    );
    assert_eq!(
        store::load_editing(),
        None,
        "an incognito session recorded a style of editing"
    );

    let mut expected = vec!["model".to_string(), "theme".to_string()];
    expected.sort();
    assert_eq!(scratch.contents(), expected);
}

/// INCOG-3: an incognito session leaves no record of itself, and none that it happened.
#[test]
fn no_session_record_is_written() {
    let scratch = Scratch::incognito("sessions");

    let mut handle = Handle::begin(&scratch.project);
    let conversation = {
        let mut conversation = bravebot_agent::Conversation::new();
        conversation.push(bravebot_aichat::protocol::Message::user(
            "something private",
        ));
        conversation.push(bravebot_aichat::protocol::Message::assistant("done"));
        conversation
    };

    handle.save(
        "something private",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 1,
            tokens: 1_200,
            spend: &BTreeMap::new(),
            timing: &BTreeMap::new(),
            model: None,
            todos: &BTreeMap::new(),
            asides: &[],
            trust: &bravebot_core::trust::TrustStore::new(),
            programs: &bravebot_core::programs::TrustedPrograms::from_iter([]),
            directories: &[],
            manifest: None,
        },
    );

    assert_eq!(
        handle.resumable(),
        None,
        "an incognito session offered itself for resuming, so something was written"
    );
    assert!(
        !scratch.own_directory().exists(),
        "an incognito session created {}, which records that a session happened at all",
        scratch.own_directory().display()
    );
}

/// INCOG-3: naming a session records nothing either.
///
/// Separate because the title takes its own path through the writer (read, amend, rewrite), and a
/// guard on `save` alone would leave it open.
#[test]
fn naming_a_session_records_nothing() {
    let scratch = Scratch::incognito("titles");

    let mut handle = Handle::begin(&scratch.project);
    handle.rename("a title worth keeping");

    assert!(
        !scratch.own_directory().exists(),
        "naming an incognito session wrote something"
    );
}

/// INCOG-4: the audit trail of an incognito session stays in the session.
///
/// The trail says which gates ran and on what paths. It is not content, but it is a record of a
/// session having happened and what it touched, which is the thing this mode is for not leaving.
#[test]
fn no_audit_trail_is_written() {
    let scratch = Scratch::incognito("audit");

    let handle = Handle::begin(&scratch.project);
    handle.append_audit(
        1,
        &[bravebot_tui::audit::Stamped {
            at: 1_700_000_000,
            event: bravebot_core::event::Event::GatePassed {
                gate: "a-gate",
                detail: "a decision worth recording".to_string(),
            },
        }],
    );

    assert!(
        !scratch.own_directory().exists(),
        "an incognito session wrote an audit trail"
    );
}

/// UPDATE-8: an incognito session still reads what an ordinary one wrote down about updating.
///
/// The mode is about what survives a session rather than about what it may know, and a private
/// session that stopped telling somebody their bravebot was superseded would be crippled rather
/// than private. Nothing new is recorded and the answer already there is left as it was: asking
/// again is decided by whether this session may write at all, and the write itself resolves the
/// directory through the writing answer, as every other write to it does.
#[test]
fn the_answer_about_updating_is_still_read() {
    let scratch = Scratch::incognito("updates");
    // The binary this test is running as, which is what makes the seeded record name it.
    let running = std::env::current_exe().expect("the test binary");
    scratch.seed("installed-by", &format!("{}\n", running.display()));
    scratch.seed("update-check", "1700000000\treleases\t99.0.0\n");

    let said = bravebot_tui::update::at_startup().expect("a version this far ahead is an update");
    assert!(
        said.contains("99.0.0"),
        "the recorded answer was not read back: {said}"
    );

    let mut expected = vec!["installed-by".to_string(), "update-check".to_string()];
    expected.sort();
    assert_eq!(
        scratch.contents(),
        expected,
        "an incognito session recorded something about updating"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.own_directory().join("update-check"))
            .expect("the seeded file"),
        "1700000000\treleases\t99.0.0\n",
        "an incognito session overwrote the recorded answer"
    );
}
