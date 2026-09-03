//! Where `~/.bravebot` is.
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

    let dir = std::env::temp_dir().join(format!("bravebot-agent-home-{name}"));
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
        assert_eq!(
            bravebot_agent::home::directory(),
            Some(home.join(".bravebot"))
        );
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

    let found = bravebot_agent::home::directory();

    if let Some(value) = previous {
        unsafe { std::env::set_var("HOME", value) };
    }
    assert_eq!(found, None, "a missing home invented a directory");
}

/// An empty HOME is a misconfigured environment, not the filesystem root. Joining onto it would
/// put the user's own files in `/.bravebot`.
#[test]
fn an_empty_home_is_treated_as_no_home_at_all() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let previous = std::env::var_os("HOME");
    // SAFETY: single-threaded within the lock, and restored before returning.
    unsafe { std::env::set_var("HOME", "") };

    let found = bravebot_agent::home::directory();

    match previous {
        Some(value) => unsafe { std::env::set_var("HOME", value) },
        None => unsafe { std::env::remove_var("HOME") },
    }
    assert_eq!(found, None, "an empty home was joined onto anyway");
}

/// A batch imported from the wrong channel must be reported, not passed over in silence.
///
/// A credential only verifies against the deployment that signed it, so a staging batch cannot be
/// spent against the production endpoint. Skipping it quietly is the silent downgrade PREM-8 exists
/// to prevent: the request goes out on the free tier, the endpoint answers a premium model name with
/// a weaker model rather than an error, and the subscription the user is paying for goes unused with
/// nothing on screen to connect the two.
#[test]
fn a_subscription_imported_for_another_environment_is_reported() {
    with_temp_home("environment-mismatch", |_| {
        let batch = bravebot_skus::StoredCredentials {
            order_id: "aaaaaaaa-1111-4222-8333-444444444444".to_string(),
            environment: bravebot_skus::Environment::Staging,
            item_id: "b7114ccc-b3a5-4951-9a5d-8b7a28731111".to_string(),
            issuer: "brave.com?sku=brave-leo-premium".to_string(),
            credentials: vec![bravebot_skus::store::Credential {
                unblinded: "token".to_string(),
                valid_from: "2026-08-22T00:00:00".to_string(),
                valid_to: "2099-08-23T00:00:00".to_string(),
                spent: false,
                rfc: true,
            }],
        };
        bravebot_skus::store::save(&batch).expect("a write into the scratch home");

        let discovery =
            bravebot_agent::ImportedSubscription::discover("https://ai-chat.bsg.brave.com");

        let complaint = discovery
            .complaint()
            .expect("a staging batch cannot be spent on production, and must say so");
        // Which environment it holds and what to do, since "premium is off" leaves nothing to act on.
        assert!(complaint.contains("staging"), "{complaint}");
        assert!(complaint.contains("import-leo-creds"), "{complaint}");
        assert!(discovery.found().is_none(), "nothing spendable");
    });
}
