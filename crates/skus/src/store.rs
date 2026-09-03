//! Keeping the imported credentials in a mode-0600 file under `~/.bravebot`.
//!
//! # One batch, whichever channel it came from
//!
//! There is a single file, so importing from Nightly replaces what was imported from Stable. The
//! channel says where to *read the order id from*, which is a fact about the machine's browsers, not
//! about the agent: one person's subscription is one subscription however many Brave builds they
//! have installed, and every import registers this same install as a device against it.
//!
//! Keeping one per channel implied a choice the user never made. It also meant a stale batch from a
//! channel long since uninstalled sat there being reported, and a re-import after switching channels
//! left the old file to be picked up by whichever load happened to reach it first.
//!
//! # Why not the OS keychain
//!
//! These are bearer secrets, so the keychain looks like the obvious home for them, and it was that
//! for a while. It was the wrong call on both halves of the trade.
//!
//! The browser these are imported from keeps the same secret in a plain preference. `skus.state`
//! and `brave.ai_chat.premium_credential_cache` are unencrypted JSON in the profile, and nothing
//! under brave-core's `components/skus` or `components/ai_chat` references OSCrypt for them. So a
//! keychain here guarded a copy of something already readable in the file the copy came from.
//!
//! Nor does it hold against the threat it was written for, which was a program `run` launches
//! reading the file. Those are deliberately unconfined (RUN-10), and the AWS credentials that sign
//! every model request are cached by the `aws` CLI in plain 0600 JSON, so anything able to read a
//! file here can already take the larger secret. Guarding the smaller one behind the keychain
//! bought a password dialog, not a boundary: [`Wallet`] holds the whole decrypted batch in memory
//! for the session either way.
//!
//! What it did cost was availability. The keychain crate builds one Linux backend, the D-Bus
//! Secret Service, so a machine reached over SSH with no desktop session had no store to open at
//! all and every such user was silently on the free tier.
//!
//! # Why the whole batch is stored, not one cookie
//!
//! A time-limited-v2 credential is single-use. Presenting one to the backend spends it, so what is
//! stored is the batch the server signed, and a request takes the next unspent one. Caching a
//! ready-made cookie value would mean replaying a spent credential on the second request.

use crate::device::Registration;
use std::path::PathBuf;

/// The directory inside the user's home the file is kept in.
const DIRECTORY: &str = ".bravebot";

/// The file itself. One, not one per channel: see the module documentation.
const FILE: &str = "leo-premium.json";

#[derive(Debug)]
pub enum StoreError {
    /// Nothing has been imported yet.
    NotFound,
    /// The file exists and could not be read or written.
    Unusable { detail: String },
    /// The file exists but is not what this version writes.
    Malformed { detail: String },
    /// Every credential valid now has been spent.
    Exhausted,
    /// The batch's last validity window has closed.
    ///
    /// Separate from [`StoreError::Exhausted`] because it is the usual way a batch stops working,
    /// and it typically happens with most of the batch never used.
    Expired { until: String, unspent: usize },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("no imported Leo subscription found"),
            Self::Unusable { detail } => {
                write!(f, "the stored credentials could not be read: {detail}")
            }
            Self::Malformed { detail } => {
                write!(f, "the stored credentials are unusable: {detail}")
            }
            Self::Exhausted => f.write_str(
                "every credential valid today has been spent; run `bravebot import-leo-creds` again",
            ),
            Self::Expired { until, unspent } => write!(
                f,
                "the imported credentials expired at {until} with {unspent} never used; \
                 run `bravebot import-leo-creds` again"
            ),
        }
    }
}

impl std::error::Error for StoreError {}

/// A signed credential batch, as stored.
///
/// Serialised as JSON rather than a bespoke encoding because a readable shape is one less thing to
/// get wrong when the format changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCredentials {
    /// The order the batch belongs to, so a re-import can refresh in place.
    pub order_id: String,
    /// Which service issued this batch, so a refill goes back to the same one.
    ///
    /// Stored rather than recomputed because the browser it was imported from may be gone by then,
    /// and a credential minted against the wrong environment cannot be used.
    pub environment: crate::Environment,
    /// The item the credentials are for.
    pub item_id: String,
    /// `merchant?sku=` string the presentation signs over.
    pub issuer: String,
    /// The unblinded credentials, each usable once.
    pub credentials: Vec<Credential>,
}

/// One single-use credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    /// Base64 unblinded token, from which the verification key is derived.
    pub unblinded: String,
    /// Start of the window this credential is valid in, as the server stated it.
    pub valid_from: String,
    /// End of that window.
    pub valid_to: String,
    /// Whether this one has already been presented.
    pub spent: bool,
    /// Which key derivation this token was blinded with.
    ///
    /// Stored per credential because the two derivations yield different verification keys, and
    /// picking the wrong one produces a signature the server rejects with nothing to explain it.
    pub rfc: bool,
}

impl StoredCredentials {
    /// How many credentials remain unspent.
    pub fn remaining(&self) -> usize {
        self.credentials.iter().filter(|c| !c.spent).count()
    }

    /// The index of the next credential usable at `now`.
    ///
    /// `now` is passed in rather than read from the clock so the choice is testable, and compared
    /// as a string because the server's timestamps are fixed-width ISO 8601 in UTC, which orders
    /// lexicographically. Parsing them would add a date library to compare two strings.
    pub fn next_usable(&self, now: &str) -> Option<usize> {
        self.credentials
            .iter()
            .position(|c| !c.spent && c.valid_from.as_str() <= now && now < c.valid_to.as_str())
    }

    /// Whether every credential's window has closed by `now`.
    ///
    /// Distinct from being spent, and the distinction matters because it is the common case: a
    /// batch covers a few daily windows, so it usually stops working with most of it never used.
    /// Reporting that as "used up" would send someone looking for heavy usage that did not happen.
    pub fn expired(&self, now: &str) -> bool {
        !self.credentials.is_empty() && self.credentials.iter().all(|c| c.valid_to.as_str() <= now)
    }

    /// The end of the last window, which is when this batch stops being usable.
    pub fn usable_until(&self) -> Option<&str> {
        self.credentials.iter().map(|c| c.valid_to.as_str()).max()
    }
}

impl From<Registration> for StoredCredentials {
    fn from(value: Registration) -> Self {
        Self {
            order_id: value.order_id,
            environment: value.environment,
            item_id: value.item_id,
            issuer: value.issuer,
            credentials: value
                .credentials
                .into_iter()
                .map(|c| Credential {
                    unblinded: c.unblinded,
                    valid_from: c.valid_from,
                    valid_to: c.valid_to,
                    spent: false,
                    rfc: c.rfc,
                })
                .collect(),
        }
    }
}

/// The file holding the imported credentials.
///
/// `HOME` is read directly rather than through a dependency, for the same reason and with the same
/// absence of a fallback as everywhere else it is resolved: inventing a directory would put a
/// bearer secret somewhere the user never chose.
pub fn path() -> Result<PathBuf, StoreError> {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty());
    let Some(home) = home else {
        return Err(StoreError::Unusable {
            detail: "no HOME is set, so there is nowhere to keep credentials".to_string(),
        });
    };
    Ok(PathBuf::from(home).join(DIRECTORY).join(FILE))
}

/// Write the batch, replacing whatever was there.
///
/// Created 0600 before anything is written to it, rather than written and then chmod'ed: the other
/// order leaves the secret world-readable for the moment in between.
pub fn save(credentials: &StoredCredentials) -> Result<(), StoreError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let path = path()?;
    let unusable = |detail: String| StoreError::Unusable { detail };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| unusable(format!("{}: {e}", parent.display())))?;
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| unusable(format!("{}: {e}", path.display())))?;

    file.write_all(encode(credentials).as_bytes())
        .map_err(|e| unusable(format!("{}: {e}", path.display())))
}

/// Read the batch.
pub fn load() -> Result<StoredCredentials, StoreError> {
    let path = path()?;
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(StoreError::NotFound),
        Err(e) => {
            return Err(StoreError::Unusable {
                detail: format!("{}: {e}", path.display()),
            });
        }
    };
    decode(&raw)
}

/// A batch held open for a session, spending from memory.
///
/// # Why this is not read and written per request
///
/// A credential is spent on *every* model request, and a batch is hundreds of them, so writing the
/// whole file per spend would rewrite the same few hundred kilobytes several times a turn to change
/// one boolean. The batch is read once, spent from memory, and the markers are written back when
/// the session ends or when [`Wallet::flush`] is called.
///
/// The failure mode is losing spend markers if the process dies, which means a credential that was
/// presented is still recorded as unspent. That is deliberately the direction to fail in: a batch
/// is hundreds of credentials valid for days, so wasting a few is free, and the alternative
/// (recording a spend that never happened) is what runs the batch down for no benefit.
pub struct Wallet {
    batch: StoredCredentials,
    /// Where a flush writes, or `None` for a detached batch that must never be written.
    ///
    /// Resolved when the wallet is opened rather than at flush time, which is what makes a detached
    /// wallet safe: there is no path to write to, so no code path, including [`Drop`], can reach the
    /// filesystem. A boolean would leave a real destination sitting there for a later edit to use.
    destination: Option<PathBuf>,
    /// Whether anything has been spent since the last write.
    dirty: bool,
}

impl Wallet {
    /// Read the batch, reading the file at most once.
    pub fn open() -> Result<Self, StoreError> {
        Ok(Self {
            batch: load()?,
            destination: Some(path()?),
            dirty: false,
        })
    }

    /// Hold a batch that is already in hand, with no file behind it.
    ///
    /// For tests, including those in crates above this one, which is why it is public. A test must
    /// never write to the real store: it would overwrite the credentials of whoever ran it.
    ///
    /// The result is detached, so spending and flushing behave normally but nothing is ever
    /// written, not even by [`Drop`].
    pub fn detached(batch: StoredCredentials) -> Self {
        Self {
            batch,
            destination: None,
            dirty: false,
        }
    }

    /// Take the next credential usable at `now`, marking it spent in memory.
    pub fn spend(&mut self, now: &str) -> Result<Spent, StoreError> {
        let index = match self.batch.next_usable(now) {
            Some(index) => index,
            // Nothing usable is normal rather than exceptional: a batch covers a few daily windows
            // and stops working when the last one closes, usually with most of it unspent. So the
            // two cases are reported apart, and the caller refills rather than giving up.
            None if self.batch.expired(now) => {
                return Err(StoreError::Expired {
                    until: self.batch.usable_until().unwrap_or("unknown").to_string(),
                    unspent: self.batch.remaining(),
                });
            }
            None => return Err(StoreError::Exhausted),
        };

        self.batch.credentials[index].spent = true;
        self.dirty = true;

        Ok(Spent {
            credential: self.batch.credentials[index].clone(),
            issuer: self.batch.issuer.clone(),
            remaining: self.batch.remaining(),
        })
    }

    /// The order this batch belongs to, so a refill knows what to register against.
    pub fn order_id(&self) -> &str {
        &self.batch.order_id
    }

    /// Which service issued this batch, so a refill returns to the same one.
    pub fn environment(&self) -> crate::Environment {
        self.batch.environment
    }

    /// Replace the batch with a freshly issued one, keeping the same destination.
    ///
    /// Marked dirty so the new batch is written even though nothing has been spent from it yet:
    /// losing it would mean minting another on the next run for no reason.
    pub fn refill(&mut self, batch: StoredCredentials) {
        self.batch = batch;
        self.dirty = true;
    }

    /// Write the spent markers back, if any.
    ///
    /// A no-op when nothing was spent, so an idle session never writes at all, and a no-op for a
    /// detached batch, which has nowhere to write.
    pub fn flush(&mut self) -> Result<(), StoreError> {
        if self.destination.is_none() || !self.dirty {
            return Ok(());
        }
        save(&self.batch)?;
        self.dirty = false;
        Ok(())
    }

    pub fn remaining(&self) -> usize {
        self.batch.remaining()
    }
}

/// Writes the spent markers back, so a session that ends normally does not replay credentials.
///
/// Errors are dropped: this runs during teardown where there is nothing useful to do with one, and
/// the consequence is only that some spent credentials look unspent next time.
impl Drop for Wallet {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

/// A credential taken out of the store, already recorded as used.
#[derive(Debug, Clone)]
pub struct Spent {
    pub credential: Credential,
    /// The issuer string this credential's presentation signs over.
    pub issuer: String,
    /// How many are left, so a caller can warn before the batch runs out.
    pub remaining: usize,
}

/// Forget the imported batch.
pub fn clear() -> Result<(), StoreError> {
    let path = path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        // Already absent is the outcome asked for, not a failure.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StoreError::Unusable {
            detail: format!("{}: {e}", path.display()),
        }),
    }
}

fn encode(credentials: &StoredCredentials) -> String {
    serde_json::json!({
        "version": 1,
        "order_id": credentials.order_id,
        "environment": credentials.environment.as_str(),
        "item_id": credentials.item_id,
        "issuer": credentials.issuer,
        "credentials": credentials
            .credentials
            .iter()
            .map(|c| serde_json::json!({
                "unblinded": c.unblinded,
                "valid_from": c.valid_from,
                "valid_to": c.valid_to,
                "spent": c.spent,
                "rfc": c.rfc,
            }))
            .collect::<Vec<_>>(),
    })
    .to_string()
}

fn decode(raw: &str) -> Result<StoredCredentials, StoreError> {
    // An entry can exist holding nothing, if a write was interrupted partway. Reported as absent
    // rather than malformed, because the fix is the same as never having imported: run the import.
    if raw.trim().is_empty() {
        return Err(StoreError::NotFound);
    }

    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| StoreError::Malformed {
            detail: format!("not valid JSON: {e}"),
        })?;

    let field = |name: &str| -> Result<String, StoreError> {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| StoreError::Malformed {
                detail: format!("missing '{name}'"),
            })
    };

    let credentials = value
        .get("credentials")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| StoreError::Malformed {
            detail: "missing 'credentials'".to_string(),
        })?
        .iter()
        .map(|c| {
            let text = |name: &str| {
                c.get(name)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            Credential {
                unblinded: text("unblinded"),
                valid_from: text("valid_from"),
                valid_to: text("valid_to"),
                spent: c
                    .get("spent")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                // Every batch this writes is blinded with the rfc derivation, so that is the
                // reading for an entry that predates the field being recorded.
                rfc: c
                    .get("rfc")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
            }
        })
        .collect::<Vec<_>>();

    if credentials.iter().any(|c| c.unblinded.is_empty()) {
        return Err(StoreError::Malformed {
            detail: "a credential has no token".to_string(),
        });
    }

    Ok(StoredCredentials {
        order_id: field("order_id")?,
        // Entries written before this was recorded were all production, which was the only
        // environment reachable then.
        environment: value
            .get("environment")
            .and_then(serde_json::Value::as_str)
            .and_then(crate::Environment::of_name)
            .unwrap_or(crate::Environment::Production),
        item_id: field("item_id")?,
        issuer: field("issuer")?,
        credentials,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One lock for the whole file, not one per test.
    ///
    /// `HOME` is process-wide and these tests run in threads of one process, so every test that
    /// touches it contends for the same thing. A mutex declared inside each function would be a
    /// different mutex, and two tests would then see each other's home.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Point `HOME` at a scratch directory, so no test can read or write the developer's own
    /// credentials.
    fn with_temp_home<T>(name: &str, body: impl FnOnce() -> T) -> T {
        let dir = std::env::temp_dir().join(format!("bravebot-skus-{name}"));
        with_home(Some(dir), body)
    }

    fn with_no_home<T>(body: impl FnOnce() -> T) -> T {
        with_home(None, body)
    }

    fn with_home<T>(dir: Option<PathBuf>, body: impl FnOnce() -> T) -> T {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(dir) = &dir {
            let _ = std::fs::remove_dir_all(dir);
            std::fs::create_dir_all(dir).expect("scratch home");
        }

        let previous = std::env::var_os("HOME");
        // SAFETY: single-threaded within the lock, and restored before returning.
        match &dir {
            Some(dir) => unsafe { std::env::set_var("HOME", dir) },
            None => unsafe { std::env::remove_var("HOME") },
        }

        let result = body();

        match previous {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        if let Some(dir) = &dir {
            let _ = std::fs::remove_dir_all(dir);
        }
        result
    }

    fn batch() -> StoredCredentials {
        StoredCredentials {
            order_id: "aaaaaaaa-1111-4222-8333-444444444444".to_string(),
            environment: crate::Environment::Production,
            item_id: "b7114ccc-b3a5-4951-9a5d-8b7a28731111".to_string(),
            issuer: "brave.com?sku=brave-leo-premium".to_string(),
            credentials: vec![
                Credential {
                    unblinded: "token-one".to_string(),
                    valid_from: "2026-08-22T00:00:00".to_string(),
                    valid_to: "2026-08-23T00:00:00".to_string(),
                    spent: false,
                    rfc: true,
                },
                Credential {
                    unblinded: "token-two".to_string(),
                    valid_from: "2026-08-23T00:00:00".to_string(),
                    valid_to: "2026-08-24T00:00:00".to_string(),
                    spent: false,
                    rfc: true,
                },
            ],
        }
    }

    /// The reason Wallet exists: spending must not write, because a credential is spent per model
    /// request and each write rewrites the whole batch.
    ///
    /// Asserted through the dirty flag, which is what decides whether a write happens at all.
    #[test]
    fn spending_does_not_write_until_asked_to() {
        let mut wallet = Wallet::detached(batch());
        assert!(!wallet.dirty, "a freshly opened batch has nothing to write");

        wallet
            .spend("2026-08-22T12:00:00")
            .expect("a usable credential");
        assert!(wallet.dirty, "a spend must be recorded for the next flush");
        assert_eq!(wallet.remaining(), 1);
    }

    /// A session that spends nothing must never write, so opening the agent and not using premium
    /// leaves the stored batch untouched.
    #[test]
    fn a_session_that_spends_nothing_never_writes() {
        let mut wallet = Wallet::detached(batch());
        wallet.flush().expect("flushing nothing is a no-op");
        assert!(!wallet.dirty);
    }

    /// A detached batch must have no destination at all, which is what lets these tests run
    /// anywhere: a test that could reach the real store would overwrite the credentials of whoever
    /// ran it.
    #[test]
    fn a_detached_batch_has_nowhere_to_write() {
        let mut wallet = Wallet::detached(batch());
        assert!(wallet.destination.is_none());

        wallet
            .spend("2026-08-22T12:00:00")
            .expect("a usable credential");
        assert!(wallet.dirty, "the spend is recorded in memory");

        // Flushing a dirty detached batch is still a no-op, so neither this nor Drop can write.
        wallet.flush().expect("a detached flush cannot fail");
        assert!(wallet.dirty, "and it stays unwritten");
    }

    /// Two spends in one session must hand out different credentials: the whole batch is held in
    /// memory, so an index that did not advance would replay the same one every request.
    #[test]
    fn consecutive_spends_hand_out_different_credentials() {
        let mut batch = batch();
        // Both windows cover the same moment, so the only thing separating them is the spent mark.
        batch.credentials[1].valid_from = batch.credentials[0].valid_from.clone();
        batch.credentials[1].valid_to = batch.credentials[0].valid_to.clone();

        let mut wallet = Wallet::detached(batch);
        let first = wallet.spend("2026-08-22T12:00:00").expect("first");
        let second = wallet.spend("2026-08-22T12:00:00").expect("second");

        assert_ne!(first.credential.unblinded, second.credential.unblinded);
        assert_eq!(second.remaining, 0);
    }

    /// Once every credential in the window is spent, further requests must be refused rather than
    /// replaying one the server has already seen.
    #[test]
    fn spending_past_the_end_of_the_batch_is_refused() {
        let mut wallet = Wallet::detached(batch());
        wallet
            .spend("2026-08-22T12:00:00")
            .expect("the one usable credential");
        assert!(matches!(
            wallet.spend("2026-08-22T12:00:00"),
            Err(StoreError::Exhausted)
        ));
    }

    #[test]
    fn a_batch_survives_a_round_trip_through_the_stored_form() {
        assert_eq!(decode(&encode(&batch())).unwrap(), batch());
    }

    /// One subscription means one stored batch: importing from another channel must replace what
    /// was there, not sit beside it. Two files meant a stale batch from an uninstalled channel kept
    /// being reported, and a load picking whichever it reached first.
    #[test]
    fn importing_again_replaces_the_previous_batch() {
        with_temp_home("replace", || {
            save(&batch()).expect("a write");

            // A second import, as switching channels produces: same order, different tokens.
            let mut second = batch();
            second.credentials[0].unblinded = "from-the-other-channel".to_string();
            save(&second).expect("a second write");

            let loaded = load().expect("a read");
            assert_eq!(loaded, second, "the newer import must win");
            assert_eq!(
                loaded.credentials.len(),
                second.credentials.len(),
                "the batches must not have accumulated"
            );
        });
    }

    /// The point of moving off the keychain: a batch must survive a write and read back with no
    /// secret store involved, on a machine with no desktop session.
    #[test]
    fn a_batch_written_to_the_file_is_read_back() {
        with_temp_home("roundtrip", || {
            assert!(matches!(load(), Err(StoreError::NotFound)));

            save(&batch()).expect("a write");
            assert_eq!(load().expect("a read"), batch());
        });
    }

    /// The file holds a bearer secret, so it must not be readable by other users on the machine.
    /// Checked on the real file rather than trusted from the open flags, since an existing file
    /// keeps its own mode and the truncating reopen is the easy way to lose this.
    #[test]
    fn the_file_is_not_readable_by_anyone_else() {
        with_temp_home("mode", || {
            use std::os::unix::fs::PermissionsExt;

            save(&batch()).expect("a write");
            // Written twice: the second open finds the file already there, which is the case that
            // would silently keep a wider mode set by something else.
            save(&batch()).expect("a second write");

            let path = path().expect("a home");
            let mode = std::fs::metadata(&path)
                .expect("the file")
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o077,
                0,
                "group or other can reach {}",
                path.display()
            );
        });
    }

    /// Discarding an import must remove the secret from disk, and asking twice is not an error:
    /// `--forget` on a machine that never imported is a request that is already satisfied.
    #[test]
    fn forgetting_removes_the_file_and_is_repeatable() {
        with_temp_home("forget", || {
            save(&batch()).expect("a write");
            let path = path().expect("a home");
            assert!(path.exists());

            clear().expect("a clear");
            assert!(!path.exists(), "the secret is still on disk");
            clear().expect("clearing nothing is not an error");
        });
    }

    /// The store is under the user's own directory, not the working directory, so credentials do
    /// not depend on where the agent was started from.
    #[test]
    fn the_file_lives_in_the_users_own_directory() {
        with_temp_home("location", || {
            let path = path().expect("a home");
            let home = std::env::var("HOME").expect("a home");
            assert!(path.starts_with(&home), "{}", path.display());
            assert!(path.starts_with(PathBuf::from(&home).join(DIRECTORY)));
        });
    }

    /// With no home there is nowhere a secret belongs, and inventing one would write it somewhere
    /// the user never chose. Reported rather than treated as nothing imported, since a paid
    /// subscription silently unused is what PREM-8 exists to prevent.
    #[test]
    fn no_home_directory_is_reported_rather_than_guessed() {
        with_no_home(|| {
            assert!(matches!(path(), Err(StoreError::Unusable { .. })));
            assert!(matches!(load(), Err(StoreError::Unusable { .. })));
        });
    }

    /// A file holding something another version wrote must be reported, not read as absent: the
    /// remedy is re-importing, and treating it as absent would drop the user to the free tier
    /// without a word.
    #[test]
    fn a_file_that_is_not_json_is_reported_when_loaded() {
        with_temp_home("garbage", || {
            let path = path().expect("a home");
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
            std::fs::write(&path, "not json at all").expect("a write");

            assert!(matches!(load(), Err(StoreError::Malformed { .. })));
        });
    }

    /// A credential is single-use, so the one presented must be valid *now*: a batch covers
    /// months of daily windows and most of it is not usable on any given day.
    #[test]
    fn the_next_usable_credential_is_the_one_valid_at_that_moment() {
        let batch = batch();
        assert_eq!(batch.next_usable("2026-08-22T12:00:00"), Some(0));
        assert_eq!(batch.next_usable("2026-08-23T12:00:00"), Some(1));
    }

    #[test]
    fn a_spent_credential_is_never_offered_again() {
        let mut batch = batch();
        batch.credentials[0].spent = true;
        assert_eq!(batch.next_usable("2026-08-22T12:00:00"), None);
        assert_eq!(batch.remaining(), 1);
    }

    /// Before the first window and after the last, there is nothing to present.
    #[test]
    fn a_moment_outside_every_window_yields_no_credential() {
        let batch = batch();
        assert_eq!(batch.next_usable("2026-08-21T23:59:59"), None);
        assert_eq!(batch.next_usable("2026-09-01T00:00:00"), None);
    }

    /// The end of a window is exclusive, so the credential that expires exactly now is not used.
    #[test]
    fn a_window_does_not_include_its_own_end() {
        let batch = batch();
        assert_eq!(batch.next_usable("2026-08-23T00:00:00"), Some(1));
    }

    /// The environment must survive a round trip, since a refill uses it to pick the service and
    /// the browser it came from may be gone by then.
    #[test]
    fn the_environment_survives_a_round_trip() {
        let mut staging = batch();
        staging.environment = crate::Environment::Staging;
        assert_eq!(
            decode(&encode(&staging)).unwrap().environment,
            crate::Environment::Staging
        );
    }

    /// An interrupted write can leave the entry present but empty. Reported as absent, since the
    /// remedy is the same as never having imported, and a JSON parse error here would send someone
    /// looking for corruption instead.
    #[test]
    fn an_empty_entry_is_reported_as_absent_rather_than_malformed() {
        assert!(matches!(decode("").unwrap_err(), StoreError::NotFound));
        assert!(matches!(decode("   ").unwrap_err(), StoreError::NotFound));
    }

    #[test]
    fn a_batch_that_is_not_json_is_reported_as_malformed() {
        assert!(matches!(
            decode("not json").unwrap_err(),
            StoreError::Malformed { .. }
        ));
    }

    /// A credential with no token would fail at presentation time with something obscure, so it
    /// is rejected while there is still context to report.
    #[test]
    fn a_credential_without_a_token_is_rejected_on_load() {
        let raw = serde_json::json!({
            "version": 1,
            "order_id": "o", "item_id": "i", "issuer": "x",
            "credentials": [{ "valid_from": "a", "valid_to": "b", "spent": false }],
        })
        .to_string();
        assert!(matches!(
            decode(&raw).unwrap_err(),
            StoreError::Malformed { .. }
        ));
    }

    #[test]
    fn an_entry_missing_its_order_is_reported_as_malformed() {
        let raw = serde_json::json!({ "version": 1, "credentials": [] }).to_string();
        assert!(matches!(
            decode(&raw).unwrap_err(),
            StoreError::Malformed { .. }
        ));
    }
}
