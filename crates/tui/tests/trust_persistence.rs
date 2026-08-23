//! The trust map must outlive the process, or a restart launders untrusted content.
//!
//! The unit tests in `trust_file` cover the encoding. These cover the filesystem: a map written
//! by one session and found by the next, in the place the next one looks.
//!
//! Uses a temporary HOME so a developer's own trust map is never read or written. Getting that
//! wrong would be worse here than in the history tests: this file decides what the agent is
//! allowed to do without asking.

use bua_core::trust::TrustStore;
use bua_tui::trust_file::{self, Opening, Stored};
use std::path::Path;

/// Point HOME at a scratch directory for the duration of the closure.
///
/// Serialised through a mutex because the environment is process-wide: two of these running at
/// once would each see the other's HOME.
fn with_temp_home<T>(name: &str, body: impl FnOnce() -> T) -> T {
    use std::sync::Mutex;
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = std::env::temp_dir().join(format!("bua-trust-home-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch home");

    let previous = std::env::var_os("HOME");
    // SAFETY: single-threaded within the lock, and restored before returning.
    unsafe { std::env::set_var("HOME", &dir) };

    let result = body();

    match previous {
        Some(value) => unsafe { std::env::set_var("HOME", value) },
        None => unsafe { std::env::remove_var("HOME") },
    }
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn project() -> &'static Path {
    Path::new("/Users/someone/projects/thing")
}

/// The reason this exists. A turn writes fetched content into a trusted tree, the kernel records
/// that path as untrusted, and the session ends. The next session must not read it back as
/// trusted, which it would if the map lived only in memory.
#[test]
fn a_path_a_write_distrusted_is_still_distrusted_in_the_next_session() {
    with_temp_home("laundering", || {
        // The first session: the user trusted the directory, then a write poisoned one file.
        let mut first = TrustStore::new();
        first.trust(".");
        first.distrust("src/fetched.json");
        trust_file::save(project(), &first);

        // The second session, which knows nothing but what is on disk.
        let Stored::Rules(second) = trust_file::load(project()) else {
            panic!("the map written by the first session was not found");
        };
        assert!(second.is_trusted("src/main.rs"), "the root lost its trust");
        assert!(
            !second.is_trusted("src/fetched.json"),
            "untrusted bytes would be read back as trusted"
        );

        // And it does not ask, because asking is how that distrust would be lost.
        assert!(matches!(
            trust_file::opening(trust_file::load(project())),
            Opening::Remembered(_)
        ));
    });
}

/// A first run has nothing recorded, which is the only case the startup question is for.
#[test]
fn a_directory_nobody_has_run_in_has_nothing_recorded() {
    with_temp_home("first-run", || {
        assert_eq!(trust_file::load(project()), Stored::Nothing);
        assert!(matches!(
            trust_file::opening(trust_file::load(project())),
            Opening::Ask(_)
        ));
    });
}

/// A map that will not parse must not send the user back to a question that grants trust: the
/// rules that would have overridden the answer are the ones that could not be read.
#[test]
fn a_corrupt_map_trusts_nothing_and_is_not_asked_about() {
    with_temp_home("corrupt", || {
        let path = trust_file::path(project()).expect("a home");
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("create");
        std::fs::write(&path, "{ this is not a trust map").expect("write");

        assert_eq!(trust_file::load(project()), Stored::Unreadable);
        assert_eq!(
            trust_file::opening(trust_file::load(project())),
            Opening::Refuse
        );
    });
}

/// Two checkouts must not share a map, or trusting one would trust the other.
#[test]
fn two_directories_keep_separate_maps() {
    with_temp_home("per-directory", || {
        let one = Path::new("/Users/someone/projects/one");
        let two = Path::new("/Users/someone/projects/two");

        let mut trust = TrustStore::new();
        trust.trust(".");
        trust_file::save(one, &trust);

        assert!(matches!(trust_file::load(one), Stored::Rules(_)));
        assert_eq!(
            trust_file::load(two),
            Stored::Nothing,
            "trusting one checkout answered for another"
        );
    });
}

/// Saving twice replaces rather than accumulating, so a decision that changed does not leave the
/// old one behind it in the file.
#[test]
fn saving_again_replaces_what_was_recorded() {
    with_temp_home("replace", || {
        let mut trust = TrustStore::new();
        trust.trust(".");
        trust_file::save(project(), &trust);

        trust.distrust(".");
        trust_file::save(project(), &trust);

        let Stored::Rules(back) = trust_file::load(project()) else {
            panic!("no map on disk");
        };
        assert!(!back.is_trusted("src/main.rs"));
    });
}

/// The directory is made on demand, so a first run needs no setup.
#[test]
fn the_directory_is_created_on_first_save() {
    with_temp_home("mkdir", || {
        let path = trust_file::path(project()).expect("a home");
        assert!(!path.exists(), "the map existed already");

        let mut trust = TrustStore::new();
        trust.trust(".");
        trust_file::save(project(), &trust);

        assert!(path.exists(), "the map was not written");
    });
}

/// No home is not a failure. The session runs without a remembered map, as it did before there
/// was one to remember.
#[test]
fn no_home_directory_is_not_an_error() {
    with_temp_home("no-home", || {
        // SAFETY: single-threaded within the lock held by `with_temp_home`.
        unsafe { std::env::set_var("HOME", "") };

        assert_eq!(trust_file::path(project()), None);
        assert_eq!(trust_file::load(project()), Stored::Nothing);
        trust_file::save(project(), &TrustStore::new());
    });
}
