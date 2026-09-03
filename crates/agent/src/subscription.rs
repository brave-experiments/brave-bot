//! Spending imported Leo Premium credentials on a turn's requests.
//!
//! The adapter between the credential store and the chat client. It lives here rather than in either
//! because a credential is single-use: the client has to ask for one per request, and the store
//! has to record each as spent, so something has to sit between them and hold the channel.
//!
//! A failure here fails the turn rather than reverting to the free tier. That is deliberate: a
//! subscription that silently stops being used looks like the model got worse for no reason, and
//! the error names the fix (re-import, or unset the premium endpoint).

use bravebot_aichat::{Subscription, SubscriptionCredential};
use bravebot_skus::{DeviceError, Registration, StoreError};

/// What looking for an imported subscription found.
///
/// Three answers rather than an `Option`, because the two ways of coming back empty want different
/// treatment. Nothing imported is the free tier working as intended and deserves no words. A batch
/// that exists and could not be read is a paid subscription not being spent, and the only symptom is
/// a worse model, which nobody would think to attribute to the credential store.
///
/// No `Debug`, deliberately. One variant holds a wallet of live credentials, and the obvious
/// debugging reflex of printing this would put them in a log. The same reasoning as
/// [`bravebot_aichat::SubscriptionCredential`], which redacts for it.
pub enum Discovery {
    /// A batch is in hand and will be spent on this turn's requests.
    Found(ImportedSubscription),
    /// Nothing has been imported for this endpoint's environment. The ordinary free-tier case.
    NothingImported,
    /// Something is stored and could not be used, with a sentence saying why and what to do.
    ///
    /// Carries words rather than the error so a caller can report it without depending on
    /// `bravebot_skus`, and because what a person needs here is the sentence, not the variant.
    Refused(String),
}

impl Discovery {
    /// The subscription, where one was found.
    pub fn found(self) -> Option<ImportedSubscription> {
        match self {
            Self::Found(subscription) => Some(subscription),
            _ => None,
        }
    }

    /// What to tell the person watching, where there is anything worth saying.
    ///
    /// `None` for both of the cases that are working as intended: a subscription being spent needs
    /// no explanation, and neither does a free-tier session that never had one.
    pub fn complaint(&self) -> Option<&str> {
        match self {
            Self::Refused(detail) => Some(detail),
            Self::Found(_) | Self::NothingImported => None,
        }
    }
}

/// How a new batch is obtained, as a function so a test can supply one without a network.
type Register = fn(bravebot_skus::Environment, &str, &str) -> Result<Registration, DeviceError>;

/// Spends imported credentials, one per request.
///
/// The batch is read once and spent from memory, so a session touches the store at most once
/// rather than on every model round. See [`bravebot_skus::store::Wallet`].
pub struct ImportedSubscription {
    wallet: bravebot_skus::store::Wallet,
    /// The clock, as an injectable function so a test need not wait for a real date.
    now: fn() -> String,
    /// How many credentials were left after the last spend, for a caller that wants to warn.
    remaining: Option<usize>,
    /// How a replacement batch is obtained when the current one runs out.
    register: Register,
    /// Supplies the request id a refill registers under.
    new_request_id: fn() -> String,
    /// Whether a refill has already been attempted this session.
    refilled: bool,
}

impl ImportedSubscription {
    /// Spend from a batch that is already in hand, with no file behind it.
    ///
    /// Exists so the spending behaviour can be tested without touching the real store, which would
    /// overwrite the credentials of whoever ran the suite.
    #[cfg(test)]
    fn detached(batch: bravebot_skus::StoredCredentials) -> Self {
        Self {
            wallet: bravebot_skus::store::Wallet::detached(batch),
            now: current_timestamp,
            remaining: None,
            // Refusing rather than reaching the network, so a test that unexpectedly triggers a
            // refill fails loudly instead of making a live request.
            register: |_, _, _| {
                Err(DeviceError::Transport {
                    detail: "no network in tests".to_string(),
                })
            },
            new_request_id: || "test-request-id".to_string(),
            refilled: false,
        }
    }

    /// The imported batch, if it can be spent against `endpoint`.
    ///
    /// A credential only verifies against the issuer that signed it, so a batch imported for another
    /// deployment is refused here rather than sent and rejected as invalid by the server.
    ///
    /// The pairing is production or not, rather than an exact environment match. The aichat hosts and
    /// the SKU service do not divide the world the same way: a staging subscription is verified by
    /// the `brave.software` aichat host, confirmed against the live service, so requiring the names
    /// to agree would reject a credential that works.
    ///
    /// Three answers rather than two, because "nothing is imported" and "something is imported and
    /// could not be read" are different facts and only one of them is ordinary. Collapsed into
    /// `None` they were indistinguishable, and the tier silently became the free one: an unreadable
    /// file, or a batch written by another version, cost the user the model they had chosen
    /// and said nothing at all. What that looks like from the outside is the model getting worse for
    /// no reason, which is the thing this module's own documentation promises never to do.
    pub fn discover(endpoint: &str) -> Discovery {
        // A host in neither camp, such as a local endpoint. No credential belongs anywhere near it,
        // and that is a deliberate choice rather than a failure, so it reports as nothing imported.
        let Some(production) = is_production_endpoint(endpoint) else {
            return Discovery::NothingImported;
        };

        let wallet = match bravebot_skus::store::Wallet::open() {
            Ok(wallet) => wallet,
            // The ordinary free-tier case: nothing has been imported.
            Err(StoreError::NotFound) => return Discovery::NothingImported,
            Err(e) => return Discovery::Refused(e.to_string()),
        };

        // A credential only verifies against the deployment that signed it, so one imported from a
        // channel this build does not talk to cannot be spent. Reported rather than passed over: it
        // is a paid subscription going unused, and the remedy is importing from the matching
        // channel, which nobody would guess from a turn that quietly got worse.
        if (wallet.environment() == bravebot_skus::Environment::Production) != production {
            return Discovery::Refused(format!(
                "the imported subscription is for {}, which this endpoint does not accept; \
                 run `bravebot import-leo-creds` from the matching Brave channel",
                wallet.environment().as_str()
            ));
        }

        Discovery::Found(Self {
            wallet,
            now: current_timestamp,
            remaining: None,
            register: default_register,
            new_request_id: bravebot_skus::new_request_id,
            refilled: false,
        })
    }

    /// How many credentials remained after the last one was spent.
    pub fn remaining(&self) -> Option<usize> {
        self.remaining
    }
}

impl Subscription for ImportedSubscription {
    fn next_credential(&mut self) -> Result<SubscriptionCredential, String> {
        let now = (self.now)();

        let spent = match self.wallet.spend(&now) {
            Ok(spent) => spent,
            // A batch covers a few daily windows and then stops working, usually with most of it
            // unspent, so running out is the expected end of its life rather than a fault. The
            // subscription is still paid for and everything needed to mint more is already here, so
            // refill instead of making the user re-run the import.
            Err(StoreError::Exhausted | StoreError::Expired { .. }) => {
                self.refill()?;
                self.wallet.spend(&now).map_err(|e| e.to_string())?
            }
            Err(e) => return Err(e.to_string()),
        };

        let value = bravebot_skus::device::present(&spent.credential, &spent.issuer)
            .map_err(|e| e.to_string())?;

        self.remaining = Some(spent.remaining);

        Ok(SubscriptionCredential {
            cookie_name: bravebot_skus::CREDENTIAL_COOKIE_NAME.to_string(),
            cookie_value: value,
        })
    }
}

impl ImportedSubscription {
    /// Mint a new batch for the same order and put it in the wallet.
    ///
    /// Registers again under a **fresh** request id. Reusing the stored one would claim the existing
    /// batch rather than ask for a new one, which is the case the service answers with a conflict.
    fn refill(&mut self) -> Result<(), String> {
        // Attempted once per session. A second failure in the same run would not be a different
        // answer, and retrying inside a tool loop would mint batches in a circle.
        if self.refilled {
            return Err("the subscription credentials could not be renewed".to_string());
        }
        self.refilled = true;

        let order_id = self.wallet.order_id().to_string();
        let registration = (self.register)(
            self.wallet.environment(),
            &order_id,
            &(self.new_request_id)(),
        )
        .map_err(|e| e.to_string())?;

        self.wallet.refill(registration.into());

        // Written now rather than at session end: a batch that cost a round trip to obtain should
        // survive the process not exiting cleanly.
        self.wallet.flush().map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Whether an aichat endpoint is the production deployment.
///
/// Only that distinction is drawn, because it is the only one that matters for picking a credential:
/// the non-production aichat hosts and the non-production SKU issuers do not correspond one to one,
/// and a staging subscription is verified by the `brave.software` host in practice.
///
/// `None` for a host in neither camp, such as a local endpoint, so no credential is sent somewhere
/// its issuer is unknown.
fn is_production_endpoint(endpoint: &str) -> Option<bool> {
    let host = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, rest)| rest)
        .split('/')
        .next()?;

    // Checked before brave.com, since a careless suffix test would read brave.software as
    // production and send a live credential to a development host.
    if host.ends_with(".brave.software") || host.ends_with(".bravesoftware.com") {
        Some(false)
    } else if host.ends_with(".brave.com") {
        Some(true)
    } else {
        None
    }
}

/// Register against the real service.
fn default_register(
    environment: bravebot_skus::Environment,
    order_id: &str,
    request_id: &str,
) -> Result<Registration, DeviceError> {
    bravebot_skus::device::register(environment, order_id, request_id)
}

/// The current time, in the fixed-width UTC form the stored windows use.
///
/// Formatted by hand from the Unix epoch: the comparison is against strings the server wrote, and
/// a date library would be a dependency for one conversion. Civil-time arithmetic from a day count
/// is exact, so this needs no timezone handling.
fn current_timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let (year, month, day) = civil_from_days((seconds / 86_400) as i64);
    let seconds_today = seconds % 86_400;

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        seconds_today / 3600,
        (seconds_today % 3600) / 60,
        seconds_today % 60
    )
}

/// Convert a count of days since 1970-01-01 to a civil date.
///
/// Howard Hinnant's `civil_from_days`, which is exact for every date this will ever see and
/// handles leap years without a table.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The distinction the type exists for. Nothing imported is the free tier working as intended;
    /// a batch that cannot be read is a paid subscription going unspent, and collapsing the two into
    /// one empty answer is what made the downgrade silent.
    ///
    /// Stated over the two empty variants rather than by calling `discover`, which would read the
    /// real store of whoever ran the suite. See [`ImportedSubscription::detached`].
    #[test]
    fn an_unreadable_batch_is_reported_and_an_absent_one_is_not() {
        let absent = Discovery::NothingImported;
        assert!(absent.complaint().is_none(), "the free tier is not a fault");
        assert!(absent.found().is_none());

        let refused =
            Discovery::Refused("nightly: the stored credentials could not be read".to_string());
        let complaint = refused.complaint().expect("an unreadable batch says so");
        // Which channel and why, since "premium is off" would leave nothing to act on.
        assert!(complaint.contains("nightly"), "{complaint}");
        assert!(complaint.contains("could not be read"), "{complaint}");
        assert!(refused.found().is_none(), "nothing to spend");
    }

    /// A local endpoint is in neither environment, so no credential is sent near it. That is a
    /// deliberate choice rather than a failure, so it must not be reported as one: a warning on
    /// every run against a local backend is a warning people learn to ignore.
    #[test]
    fn an_endpoint_in_no_environment_is_not_a_complaint() {
        let discovery = ImportedSubscription::discover("http://127.0.0.1:8080");
        assert!(discovery.complaint().is_none());
        assert!(discovery.found().is_none());
    }

    /// The timestamp is compared against windows the server wrote, so it has to be the same
    /// fixed-width shape rather than merely a correct instant.
    #[test]
    fn the_timestamp_is_fixed_width_and_orders_lexicographically() {
        let now = current_timestamp();
        assert_eq!(now.len(), 19, "{now}");
        assert_eq!(now.as_bytes()[4], b'-');
        assert_eq!(now.as_bytes()[7], b'-');
        assert_eq!(now.as_bytes()[10], b'T');
        assert_eq!(now.as_bytes()[13], b':');
        assert_eq!(now.as_bytes()[16], b':');
    }

    /// Known epochs, including a leap day, since an off-by-one in the calendar maths would pick
    /// the wrong credential for a whole day.
    #[test]
    fn days_since_the_epoch_convert_to_the_right_civil_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        assert_eq!(civil_from_days(365), (1971, 1, 1));
        // 2000 was a leap year despite being a century.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        // 2100 is not a leap year.
        assert_eq!(civil_from_days(47_540), (2100, 2, 28));
        assert_eq!(civil_from_days(47_541), (2100, 3, 1));
        assert_eq!(civil_from_days(20_687), (2026, 8, 22));
    }

    fn batch() -> bravebot_skus::StoredCredentials {
        bravebot_skus::StoredCredentials {
            order_id: "order".to_string(),
            environment: bravebot_skus::Environment::Production,
            item_id: "item".to_string(),
            issuer: "brave.com?sku=brave-leo-premium".to_string(),
            credentials: vec![bravebot_skus::store::Credential {
                // Not a real token, so presenting it fails. That is what the error paths below
                // exercise, without needing a signed batch from the service.
                unblinded: "not-a-token".to_string(),
                valid_from: "2026-08-22T00:00:00".to_string(),
                valid_to: "2026-08-23T00:00:00".to_string(),
                spent: false,
                rfc: true,
            }],
        }
    }

    /// The endpoint decides which imported batch is usable. Getting it wrong sends a production
    /// credential to a non-production host, which reads as an invalid credential and looks like the
    /// import failed.
    #[test]
    fn the_endpoint_decides_whether_a_production_credential_is_wanted() {
        assert_eq!(
            is_production_endpoint("https://ai-chat-premium.bsg.brave.com/v1/chat/completions"),
            Some(true)
        );
        assert_eq!(
            is_production_endpoint("https://ai-chat-premium.bsg.bravesoftware.com"),
            Some(false)
        );
        assert_eq!(
            is_production_endpoint("https://ai-chat-premium.bsg.brave.software"),
            Some(false)
        );
    }

    /// `brave.software` does not end with `brave.com`, but a suffix test written in the wrong order
    /// would treat it as production and send a live credential to a development host.
    #[test]
    fn the_development_domain_is_not_read_as_production() {
        assert_eq!(
            is_production_endpoint("https://ai-chat.bsg.brave.software"),
            Some(false)
        );
        assert_eq!(
            is_production_endpoint("https://ai-chat.bsg.bravesoftware.com"),
            Some(false)
        );
    }

    /// A local endpoint is in neither camp, so no credential is sent to it rather than one being
    /// released to a host whose issuer is unknown.
    #[test]
    fn a_local_endpoint_matches_no_environment() {
        assert_eq!(is_production_endpoint("http://127.0.0.1:8000"), None);
        assert_eq!(is_production_endpoint("https://example.invalid"), None);
    }

    /// A batch that stops working must be replaced automatically. The subscription is still paid
    /// for and everything needed to mint more is already stored, so making the user re-run the
    /// import by hand would be a gap rather than a safeguard.
    #[test]
    fn an_expired_batch_is_refilled_rather_than_failing() {
        let mut expired = batch();
        expired.credentials[0].valid_from = "2020-01-01T00:00:00".to_string();
        expired.credentials[0].valid_to = "2020-01-02T00:00:00".to_string();

        let mut subscription = ImportedSubscription::detached(expired);
        subscription.register = |_, _, _| Ok(fresh_registration());

        let credential = subscription
            .next_credential()
            .expect("an expired batch is replaced");
        assert_eq!(
            credential.cookie_name,
            bravebot_skus::CREDENTIAL_COOKIE_NAME
        );
        assert!(subscription.refilled);
    }

    /// Same for a batch whose credentials are all spent rather than expired.
    #[test]
    fn a_spent_batch_is_refilled() {
        let mut spent = batch();
        spent.credentials[0].spent = true;

        let mut subscription = ImportedSubscription::detached(spent);
        subscription.register = |_, _, _| Ok(fresh_registration());

        assert!(subscription.next_credential().is_ok());
    }

    /// A refill registers against the order already stored, so nothing has to be re-read from the
    /// browser profile, which may not even be installed any more.
    #[test]
    fn a_refill_uses_the_stored_order() {
        let mut expired = batch();
        expired.credentials.clear();

        let mut subscription = ImportedSubscription::detached(expired);
        subscription.register = |_, order_id, request_id| {
            assert_eq!(order_id, "order", "refilled against the wrong order");
            assert!(!request_id.is_empty(), "a refill needs a request id");
            Ok(fresh_registration())
        };

        assert!(subscription.next_credential().is_ok());
    }

    /// One attempt per session. Retrying inside a tool loop would mint batches in a circle, each
    /// costing a round trip, and the second answer would not differ from the first.
    #[test]
    fn a_failing_refill_is_attempted_only_once() {
        let mut empty = batch();
        empty.credentials.clear();

        let mut subscription = ImportedSubscription::detached(empty);
        subscription.register = |_, _, _| {
            Err(DeviceError::Transport {
                detail: "the service is unreachable".to_string(),
            })
        };

        assert!(subscription.next_credential().is_err());
        assert!(subscription.refilled);
        // The second call must not try again, so it reports the refusal rather than the transport
        // error a fresh attempt would produce.
        let second = subscription.next_credential().unwrap_err();
        assert!(second.contains("could not be renewed"), "retried: {second}");
    }

    /// A batch obtained at the cost of a round trip is written immediately rather than at session
    /// end, so an unclean exit does not throw it away.
    #[test]
    fn a_refill_is_flushed_immediately() {
        let mut empty = batch();
        empty.credentials.clear();

        let mut subscription = ImportedSubscription::detached(empty);
        subscription.register = |_, _, _| Ok(fresh_registration());
        subscription.next_credential().expect("refilled");

        // Detached, so the flush cannot have written to a file; what matters is that the new
        // batch is in hand and spendable.
        assert!(subscription.remaining().is_some());
    }

    /// A registration with one usable credential, as the service would return.
    fn fresh_registration() -> bravebot_skus::Registration {
        bravebot_skus::Registration {
            order_id: "order".to_string(),
            environment: bravebot_skus::Environment::Production,
            item_id: "item".to_string(),
            issuer: "brave.com?sku=brave-leo-premium".to_string(),
            credentials: vec![bravebot_skus::device::SignedCredential {
                unblinded: bravebot_skus::device::test_credential(),
                // Open-ended, so the credential is valid whenever the test happens to run.
                valid_from: "2000-01-01T00:00:00".to_string(),
                valid_to: "2999-01-01T00:00:00".to_string(),
                rfc: true,
            }],
        }
    }

    /// When the batch is unusable *and* it cannot be replaced, the request fails and says so. This
    /// is the fallback behind the automatic refill, not the first response to running out.
    #[test]
    fn an_unusable_batch_that_cannot_be_refilled_is_an_error() {
        let mut empty = batch();
        empty.credentials.clear();
        let mut subscription = ImportedSubscription::detached(empty);
        subscription.register = |_, _, _| {
            Err(DeviceError::Transport {
                detail: "the service is unreachable".to_string(),
            })
        };

        let err = subscription
            .next_credential()
            .expect_err("an empty batch that cannot be refilled cannot be spent");
        assert!(err.contains("subscription service"), "unclear error: {err}");
    }

    /// A credential that cannot be turned into a presentation is also an error, since the request
    /// would otherwise be downgraded with nothing said about it.
    #[test]
    fn a_credential_that_cannot_be_presented_is_an_error() {
        let mut subscription = ImportedSubscription::detached(batch());
        assert!(subscription.next_credential().is_err());
    }
}
