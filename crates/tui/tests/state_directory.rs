#![cfg(unix)]
//! What `~/.bravebot` and the files written directly into it are readable by.
//!
//! These run against a real directory, redirected by `HOME`, because the claim is a mode on disk
//! and nothing short of a real file carries one. Each test reads the mode back rather than
//! trusting that the write happened where it was meant to.

use bravebot_agent::Conversation;
use bravebot_aichat::protocol::Message;
use bravebot_core::programs::TrustedPrograms;
use bravebot_core::trust::TrustStore;
use bravebot_tui::history::Entry;
use bravebot_tui::sessions::{Handle, Standing};
use bravebot_tui::store;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// Serialises the tests in this binary, since `HOME` is process-wide.
static HOME: Mutex<()> = Mutex::new(());

/// A scratch home, so a test never touches the real one.
struct Scratch {
    home: PathBuf,
    /// Dropped last, releasing `HOME` only once this test is finished with it.
    _lock: MutexGuard<'static, ()>,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let lock = HOME.lock().unwrap_or_else(|held| held.into_inner());
        // Under the workspace build directory rather than the system temporary one, which is
        // shared between users and is the wrong place to prove something about privacy in.
        let home = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-scratch")
            .join(format!("state-directory-{name}"));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("create home");
        // SAFETY: `HOME` is held for as long as this value lives, so no other test in this binary
        // is reading or writing the variable while this one owns it.
        unsafe { std::env::set_var("HOME", &home) };
        Self { home, _lock: lock }
    }

    /// The directory every claim here is about.
    fn own_directory(&self) -> PathBuf {
        self.home.join(".bravebot")
    }

    fn mode_of(&self, name: &str) -> u32 {
        let path = match name.is_empty() {
            true => self.own_directory(),
            false => self.own_directory().join(name),
        };
        std::fs::metadata(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .permissions()
            .mode()
            & 0o777
    }

    /// Leave the directory and a file in it as a build with no opinion about modes would have.
    fn as_an_older_build_left_it(&self, name: &str, contents: &str) {
        let dir = self.own_directory();
        std::fs::create_dir_all(&dir).expect("seed the directory");
        std::fs::write(dir.join(name), contents).expect("seed a file");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("loosen");
        std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o644))
            .expect("loosen");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// Every prompt anybody has typed, which is paths, branch names and whatever was pasted into one.
/// On a shared machine the umask decides who else can read that, and the umask is not this
/// program's to rely on.
#[test]
fn the_prompt_history_is_readable_only_by_its_owner() {
    let scratch = Scratch::new("history");

    store::append_history(&Entry::sent("a prompt worth keeping private", None));

    assert_eq!(scratch.mode_of("history"), 0o600);
    assert_eq!(scratch.mode_of(""), 0o700, "the directory holding it");
}

/// The same for a rewrite, which takes a different path through the store: a temporary file that
/// is renamed over the real one, so the mode has to be on the temporary file rather than applied
/// to the name afterwards.
#[test]
fn a_rewritten_history_is_readable_only_by_its_owner() {
    let scratch = Scratch::new("rewrite");

    store::save_history(&[Entry::sent("one", None), Entry::sent("two", None)]);

    assert_eq!(scratch.mode_of("history"), 0o600);
}

/// Which model, theme and editing style somebody chose says less than their history does, but it
/// is written into the same directory by the same code, and a rule that covered some of the files
/// in a directory would be one nobody could check a diff against.
#[test]
fn a_recorded_choice_is_readable_only_by_its_owner() {
    let scratch = Scratch::new("choices");

    store::save_model("claude-sonnet-4-5");
    store::save_theme("nord");
    store::save_effort(Some(bravebot_aichat::protocol::Effort::Xhigh));
    store::save_editing(bravebot_tui::vim::Editing::Vi);

    for file in ["model", "theme", "effort", "editor-mode"] {
        assert_eq!(scratch.mode_of(file), 0o600, "{file}");
    }
    assert_eq!(scratch.mode_of(""), 0o700, "the directory holding them");
}

/// A machine that has already run an older build has the directory and its files at whatever the
/// umask gave them. Creating a directory that is already there does not touch its mode, so
/// without tightening, the machines that most need this fix would never get it.
#[test]
fn a_state_directory_an_older_build_left_open_is_narrowed() {
    let scratch = Scratch::new("older-build");
    scratch.as_an_older_build_left_it("history", "1700000000\t\tan earlier prompt\n");
    assert_eq!(
        scratch.mode_of(""),
        0o755,
        "the test seeded nothing to narrow"
    );

    store::save_history(&[Entry::sent("a later prompt", None)]);

    assert_eq!(scratch.mode_of(""), 0o700);
    assert_eq!(scratch.mode_of("history"), 0o600);
}

/// Narrowing walks up from what was written as far as the state directory. Whose home this is,
/// and what else is kept in it, is the user's own business.
#[test]
fn nothing_above_the_state_directory_is_touched() {
    let scratch = Scratch::new("above");
    std::fs::set_permissions(&scratch.home, std::fs::Permissions::from_mode(0o755))
        .expect("loosen");

    store::save_model("claude-sonnet-4-5");

    let home = std::fs::metadata(&scratch.home)
        .expect("the home itself")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(home, 0o755, "the directory the state directory sits in");
}

/// A session record is written by a different path from the store's, and the tightening it had of
/// its own reached `sessions` and stopped, leaving the directory above it as the umask made it.
/// Both paths have to narrow the same directory, since either may be the first to write.
#[test]
fn writing_a_session_narrows_the_state_directory() {
    let scratch = Scratch::new("session");
    scratch.as_an_older_build_left_it("history", "1700000000\t\tan earlier prompt\n");
    assert_eq!(
        scratch.mode_of(""),
        0o755,
        "the test seeded nothing to narrow"
    );
    let project = scratch.home.join("project");
    std::fs::create_dir_all(&project).expect("create project");

    let mut conversation = Conversation::new();
    conversation.push(Message::user("a prompt worth keeping private"));
    let mut handle = Handle::begin(&project);
    handle.save(
        "a prompt worth keeping private",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 1,
            tokens: 1_200,
            spend: &BTreeMap::new(),
            timing: &BTreeMap::new(),
            model: None,
            todos: &BTreeMap::new(),
            asides: &[],
            trust: &TrustStore::new(),
            programs: &TrustedPrograms::new(),
            directories: &[],
            manifest: None,
        },
    );

    assert!(
        handle.resumable().is_some(),
        "no session was written, so this test proves nothing"
    );
    assert_eq!(scratch.mode_of(""), 0o700);
    assert_eq!(scratch.mode_of("sessions"), 0o700);
}
