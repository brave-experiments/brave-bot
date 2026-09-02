//! Configuration read from the environment, populated by direnv, falling back to
//! whatever was baked in at build time.
//!
//! The signing key is wrapped in [`Secret`] so it cannot be printed. That matters
//! more than usual here: this is a public repository, and the natural debugging
//! reflex, dumping the config, would otherwise put a live credential in a log or
//! an issue report.

use std::env;
use std::fmt;

/// Environment variable names, kept together so the set is auditable at a glance.
pub mod env_var {
    include!("env_var.rs");
}

mod obfuscate;
mod settings;

pub use settings::Settings;

pub mod bedrock;

include!(concat!(env!("OUT_DIR"), "/baked.rs"));

/// The server treats an unrecognised model as `automatic`, so that is also our
/// default rather than pinning a name that may silently stop existing.
///
/// Not offered by `GET /v1/models`, which lists concrete models only, so a picker showing the
/// server's list has to put this one back.
pub const DEFAULT_MODEL: &str = "automatic";

/// How many prompt tokens a conversation may reach before it is compacted.
///
/// Low enough to be under any window worth calling a context window, since a budget above the
/// real one never fires and the session dies of the thing compaction exists to prevent. The cost
/// of being wrong downward is a summary sooner than it was needed; the cost of being wrong upward
/// is the feature not existing. See [`Config::context_budget`].
///
/// Only a fallback now. Where the endpoint says what a model's window is, that is used instead:
/// see [`budget_for_window`]. This stands in for `automatic`, whose model is chosen per request so
/// no one window describes it, and for an entry that reports nothing.
///
/// This was 100_000, which is not under any window the backend serves, so compaction could not
/// fire: recorded sessions ended at 28,600, 34,751 and 34,800 prompt tokens having never once been
/// summarised. Chosen to sit under the smallest window on the roster rather than to suit the
/// largest, because being wrong upward costs a summary sooner than it was needed and being wrong
/// downward removes compaction altogether.
///
/// [`env_var::CONTEXT_BUDGET`] overrides both this and anything discovered.
pub const DEFAULT_CONTEXT_BUDGET: u64 = 24_000;

/// The smallest context window worth using, in prompt tokens.
///
/// Not a limit anything enforces. It exists so [`DEFAULT_CONTEXT_BUDGET`] can be checked against
/// the claim its own documentation makes, which is the claim that quietly stopped being true.
pub const SMALLEST_USEFUL_WINDOW: u64 = 32_000;

/// What the endpoint sends for a model whose window it does not know.
///
/// Not a small window: a placeholder standing where a figure would be. Taken as nothing said, since
/// a budget of one token would compact before a turn could start.
const NO_WINDOW_ADVERTISED: u64 = 1;

/// The budget to use for a model the endpoint described, or `None` to fall back.
///
/// `advertised` is what `/v1/models` reported, which is already a token count with a fifth held
/// back for the reply. It is used as it stands: there is nothing to convert, and no reserve to take
/// off a figure that arrives discounted.
///
/// A small figure is taken at its word rather than raised to something more comfortable. Some models
/// on the roster really do have windows of a few thousand tokens, and a floor that lifted them to
/// the default would put the budget above the window, which does not make compaction late but
/// removes it: every round asks, no round qualifies, and the session runs to exhaustion looking
/// exactly like one with nothing to summarise. That is the failure this budget exists to prevent, so
/// a cramped budget is the better of the two wrongs.
///
/// `None` only where nothing was advertised, which is the caller's cue to use
/// [`DEFAULT_CONTEXT_BUDGET`].
pub fn budget_for_window(advertised: Option<u64>) -> Option<u64> {
    advertised.filter(|window| *window > NO_WINDOW_ADVERTISED)
}

// Checked where it cannot be skipped. A budget above the window does not make compaction late, it
// removes it, and the removal is silent: every round asks, no round qualifies, and the session
// runs to exhaustion looking exactly like one that had nothing to summarise. A compile is a
// better place to find that out than a transcript.
const _: () = assert!(DEFAULT_CONTEXT_BUDGET < SMALLEST_USEFUL_WINDOW);

/// A value that must not be printed.
///
/// `Debug` and `Display` are deliberately redacting, and the inner value is only
/// reachable through [`Secret::expose`], a name chosen so that reading a credential
/// is visible at the call site during review.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Read the secret. Call sites should be rare and obvious.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Missing(&'static str),
    Empty(&'static str),
    InvalidEndpoint { value: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(name) => write!(
                f,
                "{name} is not set and was not built in; copy .envrc.example to .envrc and run `direnv allow`"
            ),
            Self::Empty(name) => write!(f, "{name} is set but empty"),
            Self::InvalidEndpoint { value } => write!(
                f,
                "endpoint must start with http:// or https://, got '{value}'"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Everything needed to talk to the backends this build can reach.
#[derive(Debug, Clone)]
pub struct Config {
    /// Bedrock configuration, when a settings file or the environment asked for it.
    ///
    /// Additive, and not a choice of backend: the tiers named here are offered alongside the aichat
    /// roster and the model a person picks is what decides where a request goes. The aichat fields
    /// below may still be blank, since somebody pointing the agent at their own AWS account is not
    /// required to hold Brave service credentials as well.
    pub bedrock: Option<bedrock::Bedrock>,
    /// HMAC signing key. Never transmitted; used only to sign the request digest.
    pub signing_key: Secret,
    /// Key id sent in the Authorization header. The server derives its copy of the
    /// signing key from a master seed plus this id, so the two are a matched pair.
    pub key_id: String,
    /// Base URL. The API path is appended by the client.
    pub endpoint: String,
    /// Base URL for the premium tier, when this build has one.
    ///
    /// `None` leaves premium unavailable, which is the right outcome for a build that was never
    /// given the host: falling back to the free endpoint with a subscription credential attached
    /// would send the credential somewhere it does not belong.
    pub premium_endpoint: Option<String>,
    /// Model to request when the user has not picked one. The server may substitute a different
    /// one regardless.
    pub default_model: String,
    /// How many prompt tokens one request may reach before the conversation is compacted.
    ///
    /// A guess, and it has to be one. The server reports what a request cost but never what it
    /// had room for, the default model is `automatic` and resolves per request, and there is no
    /// tokeniser here to count with. So this is a number chosen to be comfortably under the
    /// smallest window worth using, and [`env_var::CONTEXT_BUDGET`] is the way out of it for
    /// anyone whose model is smaller or much larger.
    pub context_budget: u64,
    /// Whether [`Config::context_budget`] was set by hand.
    ///
    /// A budget somebody typed outranks one the endpoint advertised: they may know something about
    /// their model that the listing does not say, and silently replacing it would make the setting
    /// look broken. Nothing reads this except [`Config::adopt_window`].
    budget_was_chosen: bool,
}

/// A value captured when this binary was built, or `None` if the build had none.
///
/// The masking is undone here rather than at startup so a credential is materialised
/// only for the variable actually being read.
fn built_in(name: &str) -> Option<String> {
    let (_, masked) = BAKED.iter().find(|(baked, _)| *baked == name)?;
    String::from_utf8(obfuscate::mask(masked)).ok()
}

/// Pick between what the environment says and what the build baked in.
fn resolve(
    name: &str,
    from_env: Option<String>,
    baked: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    match from_env {
        Some(value) if !value.trim().is_empty() => Some(value),
        // A placeholder .envrc leaves a variable blank, which is not a request to
        // override, so a built-in value still applies. With none, the blank is handed
        // on so the error reports it as empty rather than missing.
        blank => baked(name).or(blank),
    }
}

impl Config {
    /// Read configuration from the process environment, falling back to the values
    /// built into this binary.
    ///
    /// The environment wins, so a developer can point a released binary at a local
    /// backend without rebuilding it.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_env_and_settings(&Settings::load())
    }

    /// Read configuration from the environment, with a settings file underneath it.
    ///
    /// The environment wins over the file for the same reason it wins over a baked-in value: a
    /// variable someone exported for one session is the most specific thing they said, and a file
    /// that overrode it would make `AWS_PROFILE=other bravebot` do nothing.
    pub fn from_env_and_settings(settings: &Settings) -> Result<Self, ConfigError> {
        Self::from_lookup(|key| {
            resolve(key, env::var(key).ok(), built_in)
                .or_else(|| settings.get(key).map(str::to_string))
        })
    }

    /// Read configuration from an arbitrary lookup, so tests need not mutate global
    /// process state.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let bedrock = bedrock::Bedrock::from_lookup(&lookup);

        // With Bedrock configured these are not requirements. Insisting on them would mean anyone
        // pointing the agent at their own AWS account also had to hold Brave service credentials for
        // a backend they may not be entitled to, and a released binary has them baked in anyway, so
        // the demand would be satisfied by values nothing goes on to read.
        //
        // Blank rather than absent is what the rest of this reads to decide whether the Brave roster
        // is offered at all. See [`Config::serves_aichat`].
        let optional = bedrock.is_some();

        let required = |name: &'static str| -> Result<String, ConfigError> {
            match lookup(name) {
                Some(v) if !v.trim().is_empty() => Ok(v),
                _ if optional => Ok(String::new()),
                None => Err(ConfigError::Missing(name)),
                Some(_) => Err(ConfigError::Empty(name)),
            }
        };

        let signing_key = Secret::new(required(env_var::SIGNING_KEY)?);
        let key_id = required(env_var::KEY_ID)?;
        let endpoint = required(env_var::ENDPOINT)?;

        // A blank endpoint only reaches here when Bedrock is configured, where no aichat URL is
        // ever built. Checking the scheme of a value nothing will use would refuse a working
        // configuration over a field it does not have.
        if !endpoint.is_empty()
            && !(endpoint.starts_with("https://") || endpoint.starts_with("http://"))
        {
            return Err(ConfigError::InvalidEndpoint { value: endpoint });
        }

        // `automatic` for everyone, Bedrock block or not: it is the Brave backend's own default and a
        // settings block adds a roster rather than changing what answers when nobody has picked. The
        // exception is a build that cannot reach Brave at all, where `automatic` names a backend with
        // no credentials and the strongest configured tier is the only thing that can answer.
        let default_model = lookup(env_var::DEFAULT_MODEL)
            .filter(|m| !m.trim().is_empty())
            .or_else(|| match bedrock.as_ref() {
                Some(bedrock) if endpoint.is_empty() => bedrock.default_model().map(str::to_string),
                _ => None,
            })
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        // A premium host that is present but malformed is dropped rather than rejected: it only
        // matters to someone who has imported a subscription, and failing every run over it would
        // punish everyone else.
        let premium_endpoint = lookup(env_var::PREMIUM_ENDPOINT)
            .map(|value| value.trim().to_string())
            .filter(|value| value.starts_with("https://") || value.starts_with("http://"))
            .map(|value| value.trim_end_matches('/').to_string());

        // Nonsense falls back to the default rather than disabling compaction, which is what a
        // zero or an unparseable value would otherwise quietly do. A mistyped budget should cost
        // someone the setting they wanted, not the thing that keeps their session alive.
        let chosen_budget = lookup(env_var::CONTEXT_BUDGET)
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|budget| *budget > 0);

        // No Bedrock window applied here, though the block may be present. Both rosters are offered
        // at once now, so the window belongs to whichever model was picked rather than to the
        // configuration, and `adopt_window` takes it from the entry that named it. Applying the
        // Bedrock figure to a session that goes on to use a Brave model would set a budget for a
        // window that is not the one in force.
        let context_budget = chosen_budget.unwrap_or(DEFAULT_CONTEXT_BUDGET);
        let budget_was_chosen = chosen_budget.is_some();

        Ok(Self {
            bedrock,
            signing_key,
            key_id,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            premium_endpoint,
            default_model,
            context_budget,
            budget_was_chosen,
        })
    }

    /// Take the window the endpoint advertised for the model in use, where it is worth taking.
    ///
    /// Ignored when a budget was set by hand, and when the endpoint advertised nothing or something
    /// too small to work in: in both cases what is already here stands. Returns whether the budget
    /// changed, so a caller can say so once rather than every turn.
    pub fn adopt_window(&mut self, advertised: Option<u64>) -> bool {
        if self.budget_was_chosen {
            return false;
        }
        match budget_for_window(advertised) {
            Some(budget) if budget != self.context_budget => {
                self.context_budget = budget;
                true
            }
            _ => false,
        }
    }

    /// Whether the Brave backend can be reached at all.
    ///
    /// Every ordinary build can: the credentials are baked in, and a missing one is refused at
    /// startup. A build from source pointed at Bedrock is the exception, where they are allowed to be
    /// blank, and offering that roster would list models whose every request fails unsigned.
    pub fn serves_aichat(&self) -> bool {
        !self.endpoint.is_empty()
            && !self.key_id.is_empty()
            && !self.signing_key.expose().is_empty()
    }

    /// Full URL for the OpenAI-compatible chat completions endpoint.
    ///
    /// This is the v2 API: the version is inferred from the path by the server, so
    /// there is no `/v2/` route to construct.
    pub fn chat_completions_url(&self) -> String {
        format!("{}/v1/chat/completions", self.endpoint)
    }

    /// Full URL for the model listing.
    ///
    /// The free host, always: the listing reports which models are premium rather than differing
    /// between the two, and asking the premium host would spend a subscription credential to learn
    /// something the free one says for nothing.
    pub fn models_url(&self) -> String {
        format!("{}/v1/models", self.endpoint)
    }

    /// The same endpoint on the premium host, when this build has one.
    pub fn premium_chat_completions_url(&self) -> Option<String> {
        self.premium_endpoint
            .as_ref()
            .map(|base| format!("{base}/v1/chat/completions"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_env(key: &str) -> Option<String> {
        match key {
            env_var::SIGNING_KEY => Some("test-signing-key".into()),
            env_var::KEY_ID => Some("test-key-id".into()),
            env_var::ENDPOINT => Some("https://example.invalid".into()),
            _ => None,
        }
    }

    #[test]
    fn reads_a_complete_environment() {
        let config = Config::from_lookup(complete_env).unwrap();
        assert_eq!(config.key_id, "test-key-id");
        assert_eq!(config.signing_key.expose(), "test-signing-key");
    }

    #[test]
    fn the_default_model_is_automatic() {
        let config = Config::from_lookup(complete_env).unwrap();
        assert_eq!(config.default_model, DEFAULT_MODEL);
    }

    #[test]
    fn the_default_model_can_be_overridden() {
        let config = Config::from_lookup(|k| match k {
            env_var::DEFAULT_MODEL => Some("some-pinned-model".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(config.default_model, "some-pinned-model");
    }

    #[test]
    fn the_context_budget_has_a_default() {
        let config = Config::from_lookup(complete_env).unwrap();
        assert_eq!(config.context_budget, DEFAULT_CONTEXT_BUDGET);
    }

    /// The point of the whole exercise: the endpoint says what the model's window is, so the
    /// default stops standing in for it. Recorded sessions compacted at 24,000 against a model
    /// advertising 102,400, which is four times the conversation it could have held.
    #[test]
    fn an_advertised_window_replaces_the_default() {
        let mut config = Config::from_lookup(complete_env).unwrap();
        assert!(config.adopt_window(Some(102_400)));
        assert_eq!(config.context_budget, 102_400);
    }

    /// A cramped window is taken at its word. Raising it to something more comfortable would put
    /// the budget above the window, and a budget above the window does not delay compaction, it
    /// removes it: the failure this whole figure exists to prevent.
    #[test]
    fn a_small_advertised_window_is_believed_rather_than_raised() {
        let mut config = Config::from_lookup(complete_env).unwrap();
        assert!(config.adopt_window(Some(6_400)));
        assert_eq!(config.context_budget, 6_400);
    }

    /// The placeholder the endpoint sends for a model whose window it does not know. A budget of one
    /// token would compact before a turn could start, so it means "did not say".
    #[test]
    fn the_placeholder_window_is_not_adopted() {
        let mut config = Config::from_lookup(complete_env).unwrap();
        assert!(!config.adopt_window(Some(1)));
        assert_eq!(config.context_budget, DEFAULT_CONTEXT_BUDGET);
    }

    /// `automatic` resolves per request, so nothing was advertised and the default has to stand.
    #[test]
    fn nothing_advertised_leaves_the_default_alone() {
        let mut config = Config::from_lookup(complete_env).unwrap();
        assert!(!config.adopt_window(None));
        assert_eq!(config.context_budget, DEFAULT_CONTEXT_BUDGET);
    }

    /// A budget somebody typed outranks one the endpoint advertised: they may know something about
    /// their model the listing does not say, and overriding it would make the setting look broken.
    #[test]
    fn a_budget_set_by_hand_is_not_replaced_by_an_advertised_one() {
        let mut config = Config::from_lookup(|k| match k {
            env_var::CONTEXT_BUDGET => Some("4096".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert!(!config.adopt_window(Some(102_400)));
        assert_eq!(config.context_budget, 4096);
    }

    /// Adopting the budget already in force is not a change, so nothing is said about it twice.
    #[test]
    fn adopting_the_budget_already_in_use_reports_no_change() {
        let mut config = Config::from_lookup(complete_env).unwrap();
        assert!(!config.adopt_window(Some(DEFAULT_CONTEXT_BUDGET)));
    }

    /// The default is a guess at a window nobody reports, so someone running a model it is wrong
    /// for needs a way to say so without rebuilding.
    #[test]
    fn the_context_budget_can_be_overridden() {
        let config = Config::from_lookup(|k| match k {
            env_var::CONTEXT_BUDGET => Some(" 4096 ".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(config.context_budget, 4096);
    }

    /// A mistyped budget must cost someone the setting they wanted, never compaction itself: a
    /// zero or a word would otherwise read as a conversation that may grow forever, which is the
    /// failure the budget exists to prevent.
    #[test]
    fn a_budget_that_makes_no_sense_falls_back_rather_than_disabling_compaction() {
        for value in ["0", "", "   ", "lots", "-1", "1e6", "100_000"] {
            let config = Config::from_lookup(|k| match k {
                env_var::CONTEXT_BUDGET => Some(value.into()),
                other => complete_env(other),
            })
            .unwrap();
            assert_eq!(
                config.context_budget, DEFAULT_CONTEXT_BUDGET,
                "{value:?} was read as a budget"
            );
        }
    }

    /// The variable is prefixed like every other one. A bare `MODEL` is a name anything in a
    /// shared shell profile might already export, and reading it would take that as an
    /// instruction about which model to send.
    #[test]
    fn a_bare_model_variable_is_not_read() {
        let config = Config::from_lookup(|k| match k {
            "MODEL" => Some("something-else-set-this".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(config.default_model, DEFAULT_MODEL);
    }

    /// An empty value should fall back rather than sending "" and being silently
    /// reset by the server.
    #[test]
    fn an_empty_default_model_falls_back() {
        let config = Config::from_lookup(|k| match k {
            env_var::DEFAULT_MODEL => Some("   ".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(config.default_model, DEFAULT_MODEL);
    }

    /// The point of the whole feature: a settings file naming an AWS account is enough to reach a
    /// backend, without the Brave service credentials the other one needs.
    #[test]
    fn bedrock_configures_a_backend_without_any_aichat_credentials() {
        let config = Config::from_lookup(|k| match k {
            env_var::USE_BEDROCK => Some("1".into()),
            env_var::AWS_REGION => Some("us-west-2".into()),
            env_var::BEDROCK_OPUS_MODEL => Some("opus-arn".into()),
            _ => None,
        })
        .expect("bedrock alone is a working configuration");
        let bedrock = config.bedrock.expect("bedrock configured");
        assert_eq!(bedrock.default_model(), Some("opus-arn"));
    }

    /// Without Bedrock the aichat credentials are still required. Relaxing them for everyone would
    /// turn a missing key into a confusing runtime failure instead of a startup error.
    #[test]
    fn the_aichat_credentials_are_still_required_without_bedrock() {
        let err = Config::from_lookup(|k| match k {
            env_var::SIGNING_KEY => None,
            other => complete_env(other),
        })
        .unwrap_err();
        assert_eq!(err, ConfigError::Missing(env_var::SIGNING_KEY));
    }

    /// A released binary has the aichat values baked in, so a Bedrock user's config carries them
    /// whether they wanted them or not. Their presence must not switch the backend back.
    #[test]
    fn bedrock_wins_over_baked_in_aichat_values() {
        let config = Config::from_lookup(|k| match k {
            env_var::USE_BEDROCK => Some("1".into()),
            env_var::AWS_REGION => Some("us-west-2".into()),
            other => complete_env(other),
        })
        .expect("configured");
        assert!(config.bedrock.is_some());
    }

    /// A Bedrock block adds a roster; it does not move the budget. Both backends are offered at once,
    /// so a session configured for Bedrock may spend every turn on a Brave model, and a budget set
    /// from the Bedrock window would sit above the window actually in force, which does not delay
    /// compaction but removes it. The figure is adopted when a tier is picked instead.
    #[test]
    fn a_bedrock_block_does_not_move_the_budget_off_the_default() {
        let mut config = Config::from_lookup(|k| match k {
            env_var::USE_BEDROCK => Some("1".into()),
            env_var::AWS_REGION => Some("us-west-2".into()),
            env_var::BEDROCK_OPUS_MODEL => Some("opus-arn".into()),
            other => complete_env(other),
        })
        .expect("configured");
        assert_eq!(config.context_budget, DEFAULT_CONTEXT_BUDGET);
        assert!(config.adopt_window(Some(bedrock::CONTEXT_WINDOW)));
        assert_eq!(config.context_budget, bedrock::CONTEXT_WINDOW);
    }

    /// A settings block adds to what a person may choose rather than changing what answers when they
    /// have chosen nothing. A new user has Brave and a block must not silently take that away.
    #[test]
    fn a_bedrock_block_does_not_change_the_default_model() {
        let config = Config::from_lookup(|k| match k {
            env_var::USE_BEDROCK => Some("1".into()),
            env_var::AWS_REGION => Some("us-west-2".into()),
            env_var::BEDROCK_OPUS_MODEL => Some("opus-arn".into()),
            other => complete_env(other),
        })
        .expect("configured");
        assert_eq!(config.default_model, DEFAULT_MODEL);
    }

    /// The exception: with no Brave credentials, `automatic` names a backend that cannot be reached at
    /// all, so the strongest configured tier is the only thing that could answer.
    #[test]
    fn without_brave_credentials_the_default_is_the_strongest_bedrock_tier() {
        let config = Config::from_lookup(|k| match k {
            env_var::USE_BEDROCK => Some("1".into()),
            env_var::AWS_REGION => Some("us-west-2".into()),
            env_var::BEDROCK_SONNET_MODEL => Some("sonnet-arn".into()),
            _ => None,
        })
        .expect("configured");
        assert!(!config.serves_aichat());
        assert_eq!(config.default_model, "sonnet-arn");
    }

    /// A budget somebody typed outranks an advertised one: they may know which model an opaque ARN
    /// actually resolves to.
    #[test]
    fn a_budget_set_by_hand_outranks_the_assumed_bedrock_window() {
        let config = Config::from_lookup(|k| match k {
            env_var::USE_BEDROCK => Some("1".into()),
            env_var::AWS_REGION => Some("us-west-2".into()),
            env_var::CONTEXT_BUDGET => Some("4096".into()),
            other => complete_env(other),
        })
        .expect("configured");
        assert_eq!(config.context_budget, 4096);
    }

    /// A variable exported for one session is the most specific thing the person said. A file that
    /// overrode it would make `AWS_PROFILE=other bravebot` do nothing at all.
    #[test]
    fn the_environment_outranks_the_settings_file() {
        let settings = Settings::parse(r#"{"env": {"AWS_REGION": "from-the-file"}}"#);
        let chosen = resolve(env_var::AWS_REGION, Some("from-the-env".into()), |_| None)
            .or_else(|| settings.get(env_var::AWS_REGION).map(str::to_string));
        assert_eq!(chosen.as_deref(), Some("from-the-env"));
    }

    /// The file is consulted when the environment is silent, which is the case it exists for: a
    /// session started from a shell that never exported anything.
    #[test]
    fn the_settings_file_applies_when_the_environment_is_silent() {
        let settings = Settings::parse(r#"{"env": {"AWS_REGION": "from-the-file"}}"#);
        let chosen = resolve(env_var::AWS_REGION, None, |_| None)
            .or_else(|| settings.get(env_var::AWS_REGION).map(str::to_string));
        assert_eq!(chosen.as_deref(), Some("from-the-file"));
    }

    #[test]
    fn a_missing_signing_key_is_an_error() {
        let err = Config::from_lookup(|k| match k {
            env_var::SIGNING_KEY => None,
            other => complete_env(other),
        })
        .unwrap_err();
        assert_eq!(err, ConfigError::Missing(env_var::SIGNING_KEY));
    }

    /// A placeholder .envrc leaves variables present but blank, which must not be
    /// mistaken for a real credential.
    #[test]
    fn an_empty_signing_key_is_an_error() {
        let err = Config::from_lookup(|k| match k {
            env_var::SIGNING_KEY => Some(String::new()),
            other => complete_env(other),
        })
        .unwrap_err();
        assert_eq!(err, ConfigError::Empty(env_var::SIGNING_KEY));
    }

    #[test]
    fn a_missing_key_id_is_an_error() {
        let err = Config::from_lookup(|k| match k {
            env_var::KEY_ID => None,
            other => complete_env(other),
        })
        .unwrap_err();
        assert_eq!(err, ConfigError::Missing(env_var::KEY_ID));
    }

    #[test]
    fn an_endpoint_without_a_scheme_is_rejected() {
        let err = Config::from_lookup(|k| match k {
            env_var::ENDPOINT => Some("ai-chat.example.invalid".into()),
            other => complete_env(other),
        })
        .unwrap_err();
        assert!(matches!(err, ConfigError::InvalidEndpoint { .. }));
    }

    #[test]
    fn a_trailing_slash_does_not_double_up_in_the_url() {
        let config = Config::from_lookup(|k| match k {
            env_var::ENDPOINT => Some("https://example.invalid/".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(
            config.chat_completions_url(),
            "https://example.invalid/v1/chat/completions"
        );
    }

    #[test]
    fn builds_the_openai_compatible_url() {
        let config = Config::from_lookup(complete_env).unwrap();
        assert_eq!(
            config.chat_completions_url(),
            "https://example.invalid/v1/chat/completions"
        );
    }

    /// A local development endpoint over plain http is allowed.
    #[test]
    fn http_endpoints_are_allowed_for_local_development() {
        let config = Config::from_lookup(|k| match k {
            env_var::ENDPOINT => Some("http://127.0.0.1:8000".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(
            config.chat_completions_url(),
            "http://127.0.0.1:8000/v1/chat/completions"
        );
    }

    /// The premium tier is a separate deployment, so its URL must come from its own variable
    /// rather than being derived from the free host.
    #[test]
    fn the_premium_endpoint_builds_its_own_url() {
        let config = Config::from_lookup(|k| match k {
            env_var::PREMIUM_ENDPOINT => Some("https://premium.invalid".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(
            config.premium_chat_completions_url().as_deref(),
            Some("https://premium.invalid/v1/chat/completions")
        );
    }

    /// The listing lives on the free host and reports which of its entries are premium, so asking
    /// the premium host would spend a subscription credential for the same answer.
    #[test]
    fn the_model_listing_url_is_on_the_free_host() {
        let config = Config::from_lookup(|k| match k {
            env_var::PREMIUM_ENDPOINT => Some("https://premium.invalid".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(config.models_url(), "https://example.invalid/v1/models");
    }

    /// A build without the premium host must report premium as unavailable rather than quietly
    /// using the free endpoint, which would send a subscription credential to the wrong host.
    #[test]
    fn a_build_without_a_premium_endpoint_has_no_premium_url() {
        let config = Config::from_lookup(complete_env).unwrap();
        assert_eq!(config.premium_endpoint, None);
        assert_eq!(config.premium_chat_completions_url(), None);
    }

    /// A blank or schemeless premium host is discarded, not turned into a request to a host with
    /// no scheme.
    #[test]
    fn a_malformed_premium_endpoint_is_discarded() {
        for value in ["", "   ", "premium.invalid"] {
            let config = Config::from_lookup(|k| match k {
                env_var::PREMIUM_ENDPOINT => Some(value.into()),
                other => complete_env(other),
            })
            .unwrap();
            assert_eq!(config.premium_endpoint, None, "accepted '{value}'");
        }
    }

    #[test]
    fn a_trailing_slash_on_the_premium_endpoint_does_not_double_up() {
        let config = Config::from_lookup(|k| match k {
            env_var::PREMIUM_ENDPOINT => Some("https://premium.invalid/".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(
            config.premium_chat_completions_url().as_deref(),
            Some("https://premium.invalid/v1/chat/completions")
        );
    }

    #[test]
    fn secrets_are_redacted_in_debug_and_display() {
        let secret = Secret::new("live-credential");
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
        assert_eq!(format!("{secret}"), "<redacted>");
        assert!(!format!("{secret:?}").contains("live-credential"));
    }

    /// An explicit variable must win, so a released binary can be pointed at a local
    /// backend without rebuilding it.
    #[test]
    fn the_environment_overrides_a_built_in_value() {
        let chosen = resolve(
            env_var::ENDPOINT,
            Some("http://127.0.0.1:8000".into()),
            |_| Some("https://baked.invalid".into()),
        );
        assert_eq!(chosen.as_deref(), Some("http://127.0.0.1:8000"));
    }

    /// The point of baking values in: a binary started outside the source tree has no
    /// direnv and must still be configured.
    #[test]
    fn a_built_in_value_applies_when_the_environment_is_silent() {
        let chosen = resolve(env_var::ENDPOINT, None, |_| {
            Some("https://baked.invalid".into())
        });
        assert_eq!(chosen.as_deref(), Some("https://baked.invalid"));
    }

    /// A blank variable is a placeholder, not an instruction to discard the built-in
    /// value, so it must not shadow one.
    #[test]
    fn a_blank_variable_does_not_shadow_a_built_in_value() {
        let chosen = resolve(env_var::ENDPOINT, Some("   ".into()), |_| {
            Some("https://baked.invalid".into())
        });
        assert_eq!(chosen.as_deref(), Some("https://baked.invalid"));
    }

    /// With nothing baked in, a blank must still be reported as empty rather than
    /// missing, since the two suggest different fixes.
    #[test]
    fn a_blank_variable_survives_when_nothing_was_built_in() {
        let err = Config::from_lookup(|k| match k {
            env_var::SIGNING_KEY => resolve(k, Some(String::new()), |_| None),
            other => complete_env(other),
        })
        .unwrap_err();
        assert_eq!(err, ConfigError::Empty(env_var::SIGNING_KEY));
    }

    /// Masking is what keeps a baked credential out of `strings` output, and it is
    /// only usable at all if it round-trips.
    #[test]
    fn masking_round_trips_and_does_not_leave_the_value_readable() {
        let secret = "live-credential-abc123";
        let masked = obfuscate::mask(secret.as_bytes());
        assert_ne!(masked, secret.as_bytes());
        assert_eq!(obfuscate::mask(&masked), secret.as_bytes());
    }

    /// Two values sharing a prefix must not produce a shared masked prefix, or the
    /// masking would advertise the relationship.
    #[test]
    fn values_with_a_shared_prefix_do_not_share_a_masked_prefix() {
        let one = obfuscate::mask(b"prefix-aaaa");
        let two = obfuscate::mask(b"prefix-bbbbbb");
        assert_ne!(one[0], two[0]);
    }

    /// Debug-printing the whole config is the obvious debugging reflex, so it must
    /// not leak the key.
    #[test]
    fn debugging_the_config_does_not_leak_the_key() {
        let config = Config::from_lookup(complete_env).unwrap();
        let shown = format!("{config:?}");
        assert!(!shown.contains("test-signing-key"), "leaked: {shown}");
        assert!(shown.contains("redacted"));
    }
}
