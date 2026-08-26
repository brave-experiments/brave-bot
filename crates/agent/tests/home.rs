//! Where `~/.bua` is.
//!
//! `HOME` is process-wide, so these tests are the only ones in the crate that touch it and they
//! are kept in a file of their own. Everything else the agent reads from the home directory takes
//! the path as an argument, precisely so that no other test depends on the environment.

use std::path::PathBuf;
use std::sync::Mutex;

/// One lock for the whole file, not one per test.
///
/// `HOME` is process-wide, so every test here contends for the same thing. A mutex declared
/// inside each function would be a different mutex, and two tests would then be free to run at
/// once and see each other's HOME.
static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Point HOME at a scratch directory for the duration of the closure.
fn with_temp_home<T>(name: &str, body: impl FnOnce(&PathBuf) -> T) -> T {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = std::env::temp_dir().join(format!("bua-agent-home-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch home");

    let previous = std::env::var_os("HOME");
    // SAFETY: single-threaded within the lock, and restored before returning.
    unsafe { std::env::set_var("HOME", &dir) };

    let result = body(&dir);

    match previous {
        Some(value) => unsafe { std::env::set_var("HOME", value) },
        None => unsafe { std::env::remove_var("HOME") },
    }
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The directory is the user's, so it is found where the user's environment says it is, and
/// nowhere else. Guessing would mean reading files from a directory nobody chose.
#[test]
fn the_home_directory_is_the_one_the_environment_names() {
    with_temp_home("named", |home| {
        assert_eq!(bua_agent::home::directory(), Some(home.join(".bua")));
    });
}

/// Daemons and containers run without a home. That is a case to do without, never a reason to
/// refuse to start, since everything kept there is optional.
#[test]
fn an_absent_home_is_not_an_error() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let previous = std::env::var_os("HOME");
    // SAFETY: single-threaded within the lock, and restored before returning.
    unsafe { std::env::remove_var("HOME") };

    let found = bua_agent::home::directory();

    if let Some(value) = previous {
        unsafe { std::env::set_var("HOME", value) };
    }
    assert_eq!(found, None, "a missing home invented a directory");
}

/// An empty HOME is a misconfigured environment, not the filesystem root. Joining onto it would
/// put the user's own files in `/.bua`.
#[test]
fn an_empty_home_is_treated_as_no_home_at_all() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let previous = std::env::var_os("HOME");
    // SAFETY: single-threaded within the lock, and restored before returning.
    unsafe { std::env::set_var("HOME", "") };

    let found = bua_agent::home::directory();

    match previous {
        Some(value) => unsafe { std::env::set_var("HOME", value) },
        None => unsafe { std::env::remove_var("HOME") },
    }
    assert_eq!(found, None, "an empty home was joined onto anyway");
}
