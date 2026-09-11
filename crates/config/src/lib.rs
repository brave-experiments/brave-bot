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
#[cfg(test)]
mod testutil;

pub use settings::Settings;

pub mod bedrock;
pub mod provider;

include!(concat!(env!("OUT_DIR"), "/baked.rs"));

/// The server treats an unrecognised model as [`DEFAULT_MODEL`], so that is also our default rather
/// than pinning a name that may silently stop existing.
///
/// Brave-bot's own triage entry. Leo uses `automatic` for a different routing policy; requests from
/// this product identify themselves with `Brave-Product: brave-bot` so the listing and completions
/// endpoints offer this name instead.
pub const DEFAULT_MODEL: &str = "automatic-brave-bot";

/// The automatic routing name Leo uses. Remembered choices and configuration may still say it from
/// before brave-bot had its own entry; [`normalize_model`] rewrites those to [`DEFAULT_MODEL`].
const LEGACY_AUTOMATIC: &str = "automatic";

/// Rewrite a model name from an older choice or configuration to what this product sends now.
pub fn normalize_model(name: &str) -> &str {
    if name == LEGACY_AUTOMATIC {
        DEFAULT_MODEL
    } else {
        name
    }
}

/// The model a name asks for: a tier word resolved to a model that exists, anything else as written.
///
/// `opus`, `sonnet` and `haiku` name a tier rather than a model, which is what those words mean in
/// the settings file the model key is copied from. Left as the word they reach a service that has
/// never heard of them, so they are resolved: the tier's own ARN where an AWS account named one,
/// and the Brave roster's name for that tier otherwise, since every build can reach Brave.
///
/// The AWS account wins where it named the tier, because somebody who configured it asked for it
/// by name. A tier they left unset falls through to Brave rather than being guessed at, an ARN not
/// being derivable from a word, except on a build that cannot reach Brave: there a Brave name
/// reaches a service this build cannot sign for, so the strongest tier the AWS account did name is
/// the only thing that could answer.
///
/// Shared by every route to a model rather than belonging to the settings key, so a name means the
/// same model wherever it was written down. See [`Config::model_named`].
fn resolved_model(name: &str, bedrock: Option<&bedrock::Bedrock>, reaches_brave: bool) -> String {
    let named = match bedrock::Tier::from_alias(name) {
        Some(tier) => match bedrock {
            Some(bedrock) => bedrock
                .model_for(tier)
                .or_else(|| match reaches_brave {
                    false => bedrock.default_model(),
                    true => None,
                })
                .unwrap_or(tier.brave_model()),
            None => tier.brave_model(),
        },
        None => name,
    };
    normalize_model(named).to_string()
}

/// How many prompt tokens a conversation may reach before it is compacted.
///
/// Low enough to be under any window worth calling a context window, since a budget above the
/// real one never fires and the session dies of the thing compaction exists to prevent. The cost
/// of being wrong downward is a summary sooner than it was needed; the cost of being wrong upward
/// is the feature not existing. See [`Config::context_budget`].
///
/// Only a fallback now. Where the endpoint says what a model's window is, that is used instead:
/// see [`budget_for_window`]. This stands in for [`DEFAULT_MODEL`], whose model is chosen per
/// request so no one window describes it, and for an entry that reports nothing.
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
    /// Gateways the settings file configured, in the order it listed them.
    ///
    /// Additive on the same terms as [`Config::bedrock`]: the models named here are offered beside
    /// the other rosters, and the model a person picks is what decides where a request goes.
    pub providers: Vec<provider::Provider>,
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
    /// Whether the context budget was adopted from an advertised window.
    budget_was_advertised: bool,
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

/// Which model to request, from the three places that may name one.
///
/// Ranked differently from every other value, which is why it is written out rather than going
/// through [`resolve`]. The settings file sits *above* the baked-in value here: every release bakes
/// in a default model, so a `model` key ranked below it would lose on every binary anybody was
/// given, leaving a key that parses, is reported by `doctor`, and changes nothing outside a source
/// build.
///
/// An exported variable still wins, as it does everywhere else. `env.BRAVE_AI_CHAT_DEFAULT_MODEL`
/// is read last, below the baked-in value, because that block is variables and is ranked like them.
fn resolve_model(
    from_env: Option<String>,
    settings: &Settings,
    baked: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    from_env
        .filter(|value| !value.trim().is_empty())
        .or_else(|| settings.model().map(str::to_string))
        .or_else(|| baked(env_var::DEFAULT_MODEL))
        .or_else(|| settings.get(env_var::DEFAULT_MODEL).map(str::to_string))
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
    ///
    /// The `model` key is the exception, and sits above the baked-in value rather than below it.
    /// Every release bakes in a default model, so a `model` key ranked like the `env` block would
    /// lose to it on every binary anybody was given: the key would parse, be reported by `doctor`,
    /// and change nothing outside a source build. An exported variable still outranks it.
    pub fn from_env_and_settings(settings: &Settings) -> Result<Self, ConfigError> {
        Self::from_lookup_with_providers(
            |key| match key {
                env_var::DEFAULT_MODEL => resolve_model(env::var(key).ok(), settings, built_in),
                _ => resolve(key, env::var(key).ok(), built_in)
                    .or_else(|| settings.get(key).map(str::to_string)),
            },
            settings.providers().to_vec(),
        )
    }

    /// Read configuration from an arbitrary lookup, so tests need not mutate global
    /// process state.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        Self::from_lookup_with_providers(lookup, Vec::new())
    }

    /// Providers must be known before validating credentials for the other backends.
    fn from_lookup_with_providers(
        lookup: impl Fn(&str) -> Option<String>,
        providers: Vec<provider::Provider>,
    ) -> Result<Self, ConfigError> {
        let bedrock = bedrock::Bedrock::from_lookup(&lookup);

        // With Bedrock or a gateway configured these are not requirements. A user of another
        // backend need not hold Brave service credentials as well. Released binaries have them
        // baked in anyway, so the demand there would be satisfied by values nothing goes on to read.
        //
        // Blank rather than absent is what the rest of this reads to decide whether the Brave roster
        // is offered at all. See [`Config::serves_aichat`].
        let optional = bedrock.is_some() || !providers.is_empty();

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

        // A blank endpoint only reaches here when another backend is configured and no aichat URL
        // is built. Checking the scheme of a value nothing will use would refuse a working
        // configuration over a field it does not have.
        let names_a_scheme = endpoint.starts_with("https://") || endpoint.starts_with("http://");
        if !endpoint.is_empty() && !names_a_scheme {
            return Err(ConfigError::InvalidEndpoint { value: endpoint });
        }

        // [`DEFAULT_MODEL`] for everyone, Bedrock block or not: it is the Brave backend's own
        // default for this product and a settings block adds a roster rather than changing what
        // answers when nobody has picked. The exception is a build that cannot reach Brave at all,
        // where that name reaches a backend with no credentials and the strongest configured tier
        // is the only thing that can answer.
        let default_model = resolved_model(
            &lookup(env_var::DEFAULT_MODEL)
                .filter(|m| !m.trim().is_empty())
                .or_else(|| match bedrock.as_ref() {
                    Some(bedrock) if endpoint.is_empty() => {
                        bedrock.default_model().map(str::to_string)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            bedrock.as_ref(),
            !endpoint.is_empty(),
        );

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
        let budget_was_advertised = false;

        Ok(Self {
            bedrock,
            providers,
            signing_key,
            key_id,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            premium_endpoint,
            default_model,
            context_budget,
            budget_was_chosen,
            budget_was_advertised,
        })
    }

    /// Whether the context budget is a guess rather than set by hand or advertised by the model.
    pub fn budget_is_guessed(&self) -> bool {
        !self.budget_was_chosen && !self.budget_was_advertised
    }

    /// Take the window the endpoint advertised for the model in use, where it is worth taking.
    ///
    /// Ignored when a budget was set by hand, and when the endpoint advertised nothing or something
    /// too small to work in: in both cases what is already here stands. Returns whether the budget
    /// changed, so a caller can say so once rather than every turn.
    ///
    /// A window that was not advertised still stops the budget claiming to be one. What stands is
    /// then a figure taken for a model that is no longer in force, which is a guess in the sense
    /// [`Config::budget_is_guessed`] means: good enough to compact against, not good enough to
    /// state a percentage against without saying so.
    pub fn adopt_window(&mut self, advertised: Option<u64>) -> bool {
        if self.budget_was_chosen {
            return false;
        }
        let advertised = budget_for_window(advertised);
        self.budget_was_advertised = advertised.is_some();
        match advertised {
            Some(budget) if budget != self.context_budget => {
                self.context_budget = budget;
                true
            }
            _ => false,
        }
    }

    /// The gateway serving `model`, and the name to ask it for.
    ///
    /// The model names the service, so this is the whole of how a request reaches a gateway. Two
    /// spellings reach one:
    ///
    /// - `openrouter/z-ai/glm-4.6`, the provider's own id and then the name that gateway knows the
    ///   model by. Split at the first separator only, because the remainder is the gateway's to
    ///   spell and most of them contain one.
    /// - the gateway's bare name, where the block listed it. Kept because a name chosen before this
    ///   crate qualified anything is still sitting in `~/.bravebot/model`.
    ///
    /// A qualified name is tried first. It is the one spelling that says which service is meant, so
    /// a bare name that two blocks both list resolves to the first of them while a qualified one
    /// cannot be ambiguous at all.
    ///
    /// The returned name is what goes on the wire. A gateway has never heard of the id this crate
    /// files it under, so returning them together is what stops the qualified form reaching it.
    pub fn provider_for<'name>(
        &self,
        model: &'name str,
    ) -> Option<(&provider::Provider, &'name str)> {
        if let Some((id, wire)) = model.split_once('/').filter(|(_, wire)| !wire.is_empty())
            && let Some(provider) = self
                .providers
                .iter()
                .find(|provider| provider.id == id && provider.bedrock.is_none())
        {
            return Some((provider, wire));
        }
        self.providers
            .iter()
            .filter(|provider| provider.bedrock.is_none())
            .find(|provider| provider.offers(model))
            .map(|provider| (provider, model))
    }

    /// The AWS account that serves `model`, from the tier variables or from a `provider` block.
    ///
    /// The tier variables are asked first, being the older way to name a Bedrock model and the one
    /// a tier word resolves against. A name neither offers belongs to some other service.
    pub fn bedrock_for(&self, model: &str) -> Option<&bedrock::Bedrock> {
        if let Some(bedrock) = self.bedrock.as_ref().filter(|it| it.offers(model)) {
            return Some(bedrock);
        }
        self.providers
            .iter()
            .filter_map(|provider| provider.bedrock.as_ref())
            .find(|bedrock| bedrock.offers(model))
    }

    /// Every AWS account a `provider` block named, in the order the file listed them.
    pub fn bedrock_providers(
        &self,
    ) -> impl Iterator<Item = (&provider::Provider, &bedrock::Bedrock)> {
        self.providers
            .iter()
            .filter_map(|provider| provider.bedrock.as_ref().map(|bedrock| (provider, bedrock)))
    }

    /// Whether the Brave backend can be reached at all.
    ///
    /// Every ordinary build can: the credentials are baked in, and a missing one is refused at
    /// startup. A build from source pointed at Bedrock or a gateway is the exception: credentials
    /// may be blank, and offering that roster would list models whose requests fail unsigned.
    pub fn serves_aichat(&self) -> bool {
        !self.endpoint.is_empty()
            && !self.key_id.is_empty()
            && !self.signing_key.expose().is_empty()
    }

    /// The model a name asks for, resolved the way the configured model is.
    ///
    /// For every other route to a model: a flag, a remembered choice, anything a person types. The
    /// three tier words and the older spelling of the routing entry are configuration's business
    /// rather than the settings key's, so a route that resolved names itself would accept a
    /// different set of them, and one that resolved none would send a word the service has never
    /// heard of and be answered by whatever it substitutes.
    pub fn model_named(&self, name: &str) -> String {
        resolved_model(name, self.bedrock.as_ref(), !self.endpoint.is_empty())
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
    /// something the free one says for nothing. Callers send `Brave-Product: brave-bot` so the
    /// roster is the one curated for this agent rather than Leo's.
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

    #[test]
    fn a_default_budget_is_marked_as_guessed() {
        let config = Config::from_lookup(complete_env).unwrap();
        assert!(config.budget_is_guessed());
    }

    #[test]
    fn an_advertised_budget_is_not_marked_as_guessed() {
        let mut config = Config::from_lookup(complete_env).unwrap();
        assert!(config.adopt_window(Some(102_400)));
        assert!(!config.budget_is_guessed());
    }

    /// A budget that came from one model's window is not a claim about the next one, and the
    /// figure on the hint line says so. Reverting to the default instead would raise the budget
    /// above a cramped window still in force, and a budget above the window does not delay
    /// compaction, it removes it.
    #[test]
    fn a_window_nobody_advertised_leaves_an_adopted_budget_standing_and_marks_it_guessed() {
        let mut config = Config::from_lookup(complete_env).unwrap();
        assert!(config.adopt_window(Some(6_400)));
        assert!(!config.budget_is_guessed());

        assert!(!config.adopt_window(None));
        assert_eq!(config.context_budget, 6_400);
        assert!(config.budget_is_guessed());
    }

    #[test]
    fn a_budget_set_by_hand_is_not_marked_as_guessed() {
        let config = Config::from_lookup(|k| match k {
            env_var::CONTEXT_BUDGET => Some("4096".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert!(!config.budget_is_guessed());
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

    /// A gateway is additive on the same terms a Bedrock block is. Naming one adds to what a person
    /// may choose and must not decide what answers when they have chosen nothing, nor set the budget
    /// from a window belonging to a model the session may never use.
    #[test]
    fn a_provider_block_changes_neither_the_default_model_nor_the_budget() {
        let settings = Settings::parse(
            r#"{"provider": {"openrouter": {
                "options": {"baseURL": "https://openrouter.example.invalid/api/v1"},
                "models": {"z-ai/glm-4.6": {"limit": {"context": 1000000, "output": 64000}}}
            }}}"#,
        );
        let mut config =
            Config::from_lookup_with_providers(complete_env, settings.providers().to_vec())
                .expect("configured");

        assert!(config.serves_aichat());
        assert_eq!(config.signing_key.expose(), "test-signing-key");
        assert_eq!(config.key_id, "test-key-id");
        assert_eq!(config.endpoint, "https://example.invalid");
        assert_eq!(config.default_model, DEFAULT_MODEL);
        assert_eq!(config.context_budget, DEFAULT_CONTEXT_BUDGET);
        // Taken only once that model is the one in force, which is what picking it does.
        assert!(config.adopt_window(Some(1_000_000)));
    }

    #[test]
    fn a_gateway_configures_without_brave_credentials_or_a_model_roster() {
        let settings = Settings::parse(
            r#"{
            "model": "openrouter/z-ai/glm-4.6",
            "provider": {"openrouter": {"options": {"apiKey": "test-token"}}}
        }"#,
        );
        for blank in [false, true] {
            let config = Config::from_lookup_with_providers(
                |key| match key {
                    env_var::DEFAULT_MODEL => resolve_model(None, &settings, |_| None),
                    _ if blank => Some(String::new()),
                    _ => None,
                },
                settings.providers().to_vec(),
            )
            .expect("the selected gateway needs no Brave credentials");
            assert!(!config.serves_aichat());
            assert_eq!(config.default_model, "openrouter/z-ai/glm-4.6");
            let (provider, model) = config.provider_for(&config.default_model).expect("gateway");
            assert_eq!(model, "z-ai/glm-4.6");
            assert_eq!(provider.base_url, "https://openrouter.ai/api/v1");
            assert_eq!(provider.token(|_| None).as_deref(), Some("test-token"));
            assert!(provider.models.is_empty());
        }
    }

    #[test]
    fn a_gateway_can_read_its_own_environment_token_without_brave_credentials() {
        let settings = Settings::parse(
            r#"{
            "provider": {"openrouter": {"env": ["OPENROUTER_API_KEY"]}}
        }"#,
        );
        let config = Config::from_lookup_with_providers(|_| None, settings.providers().to_vec())
            .expect("gateway configured before its token is resolved");
        let provider = &config.providers[0];
        assert_eq!(
            provider
                .token(|name| match name {
                    "OPENROUTER_API_KEY" => Some("environment-token".into()),
                    _ => None,
                })
                .as_deref(),
            Some("environment-token")
        );
        // Missing gateway tokens are reported by the gateway client, not as missing Brave keys.
        assert_eq!(provider.token(|_| None), None);
        assert!(!config.serves_aichat());
    }

    #[test]
    fn an_absent_or_invalid_gateway_does_not_relax_brave_validation() {
        for text in [
            "{}",
            r#"{"provider": {}}"#,
            r#"{"provider": {"unknown": {}}}"#,
            r#"{"provider": {"unknown": {"options": {"baseURL": ""}}}}"#,
        ] {
            let settings = Settings::parse(text);
            for blank in [false, true] {
                let error = Config::from_lookup_with_providers(
                    |_| blank.then(String::new),
                    settings.providers().to_vec(),
                )
                .unwrap_err();
                assert_eq!(
                    error,
                    if blank {
                        ConfigError::Empty(env_var::SIGNING_KEY)
                    } else {
                        ConfigError::Missing(env_var::SIGNING_KEY)
                    }
                );
            }
        }
    }

    /// The two blocks reach different services and a name has to go to the one that recognises it.
    /// Routed as a gateway, an AWS model is sent OpenAI-compatible chat completions over a bearer
    /// token that does not exist, which is the gap a `provider` block for Bedrock exists to close.
    #[test]
    fn a_model_an_aws_block_named_reaches_bedrock_rather_than_a_gateway() {
        let settings = Settings::parse(
            r#"{"provider": {
                "amazon-bedrock": {
                    "options": {"region": "us-west-2", "profile": "sso"},
                    "models": {"openai.gpt-5.6-sol": {}}
                },
                "openrouter": {
                    "options": {"baseURL": "https://openrouter.ai/api/v1"},
                    "models": {"z-ai/glm-4.6": {}}
                }
            }}"#,
        );
        let config =
            Config::from_lookup_with_providers(complete_env, settings.providers().to_vec())
                .expect("configured");

        let bedrock = config
            .bedrock_for("openai.gpt-5.6-sol")
            .expect("the AWS account that serves it");
        assert_eq!(bedrock.region, "us-west-2");
        assert!(
            config.provider_for("openai.gpt-5.6-sol").is_none(),
            "an AWS model was offered to the gateway path"
        );

        // The gateway beside it is untouched by any of this.
        assert!(config.provider_for("z-ai/glm-4.6").is_some());
        assert!(config.bedrock_for("z-ai/glm-4.6").is_none());
    }

    /// A qualified name says which service is meant, and the AWS entry is not one a gateway request
    /// can be built for, so naming it must not produce one.
    #[test]
    fn a_name_qualified_by_the_aws_id_is_not_a_gateway_either() {
        let settings = Settings::parse(
            r#"{"provider": {"amazon-bedrock": {
                "options": {"region": "us-west-2"},
                "models": {"openai.gpt-5.6-sol": {}}
            }}}"#,
        );
        let config =
            Config::from_lookup_with_providers(complete_env, settings.providers().to_vec())
                .expect("configured");
        assert!(
            config
                .provider_for("amazon-bedrock/openai.gpt-5.6-sol")
                .is_none()
        );
    }

    #[test]
    fn a_gateway_does_not_hide_an_invalid_brave_endpoint() {
        let settings = Settings::parse(r#"{"provider": {"openrouter": {}}}"#);
        let error = Config::from_lookup_with_providers(
            |key| match key {
                env_var::ENDPOINT => Some("invalid".into()),
                other => complete_env(other),
            },
            settings.providers().to_vec(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ConfigError::InvalidEndpoint {
                value: "invalid".into()
            }
        );
    }

    #[test]
    fn a_gateway_keeps_bedrock_available_without_brave_credentials() {
        let settings = Settings::parse(r#"{"provider": {"openrouter": {}}}"#);
        let config = Config::from_lookup_with_providers(
            |key| match key {
                env_var::USE_BEDROCK => Some("1".into()),
                env_var::AWS_REGION => Some("us-west-2".into()),
                env_var::BEDROCK_OPUS_MODEL => Some("opus-arn".into()),
                _ => None,
            },
            settings.providers().to_vec(),
        )
        .expect("both backends configured");
        assert!(!config.serves_aichat());
        assert_eq!(config.default_model, "opus-arn");
        assert!(config.bedrock.as_ref().unwrap().offers("opus-arn"));
        assert!(config.provider_for("openrouter/z-ai/glm-4.6").is_some());
    }

    /// A gateway configured for the tests below, offering one model the block names.
    fn with_a_gateway(models: &str) -> Config {
        let settings = Settings::parse(&format!(
            r#"{{"provider": {{"openrouter": {{
                "options": {{"baseURL": "https://openrouter.example.invalid/api/v1"}},
                "models": {models}
            }}}}}}"#
        ));
        let mut config = Config::from_lookup(complete_env).expect("configured");
        config.providers = settings.providers().to_vec();
        config
    }

    /// The name says which service is meant, which is what lets a gateway offer a model it did not
    /// have to be configured with. Only the part after the id reaches the gateway: the id is this
    /// crate's own filing, and no service has heard of it.
    #[test]
    fn a_name_qualified_by_a_provider_id_names_the_gateway_and_the_model_separately() {
        let config = with_a_gateway("{}");
        let (provider, wire) = config
            .provider_for("openrouter/z-ai/glm-4.6")
            .expect("a gateway");
        assert_eq!(provider.id, "openrouter");
        assert_eq!(wire, "z-ai/glm-4.6");
    }

    /// A name chosen before this crate qualified anything is still sitting in `~/.bravebot/model`, so
    /// a bare name the block lists has to keep reaching the gateway that lists it.
    #[test]
    fn a_bare_name_the_block_lists_still_finds_its_gateway() {
        let config = with_a_gateway(r#"{"z-ai/glm-4.6": {}}"#);
        let (provider, wire) = config.provider_for("z-ai/glm-4.6").expect("a gateway");
        assert_eq!(provider.id, "openrouter");
        assert_eq!(wire, "z-ai/glm-4.6");
    }

    /// The other two rosters share this name space. A Brave slug carries no separator and a Bedrock
    /// ARN's leading segment is no provider's id, so neither is claimed by a gateway that was asked
    /// about a name belonging to somebody else.
    #[test]
    fn a_name_no_gateway_was_configured_for_reaches_no_gateway() {
        let config = with_a_gateway(r#"{"z-ai/glm-4.6": {}}"#);
        for name in [
            "claude-3-sonnet",
            DEFAULT_MODEL,
            "arn:aws:bedrock:us-west-2:1:application-inference-profile/abc",
            "moonshot/kimi-k2",
            "openrouter/",
            "openrouter",
        ] {
            assert!(
                config.provider_for(name).is_none(),
                "{name:?} reached a gateway"
            );
        }
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

    /// The point of reading the key: a file naming a model is what decides, without the variable
    /// being exported anywhere.
    #[test]
    fn a_model_in_the_settings_file_becomes_the_default() {
        let settings = Settings::parse(r#"{"model": "llama-3-8b-instruct"}"#);
        let config = Config::from_lookup(|key| match key {
            env_var::DEFAULT_MODEL => resolve_model(None, &settings, |_| None),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(config.default_model, "llama-3-8b-instruct");
    }

    /// The whole reason this value is ranked differently. Every release bakes a default model in, so
    /// a `model` key ranked below it would lose on every binary anybody was given: the key would
    /// parse, `doctor` would report it, and nothing outside a source build would change.
    #[test]
    fn a_model_in_the_settings_file_outranks_the_baked_in_one() {
        let settings = Settings::parse(r#"{"model": "opus"}"#);
        let chosen = resolve_model(None, &settings, |_| Some("automatic".into()));
        assert_eq!(chosen.as_deref(), Some("opus"));
    }

    /// A variable exported for one session is still the most specific thing the person said, here as
    /// everywhere else.
    #[test]
    fn an_exported_model_outranks_the_settings_file() {
        let settings = Settings::parse(r#"{"model": "opus"}"#);
        let chosen = resolve_model(Some("from-the-env".into()), &settings, |_| None);
        assert_eq!(chosen.as_deref(), Some("from-the-env"));
    }

    /// A blank variable is a placeholder rather than an instruction to discard what the file said.
    #[test]
    fn a_blank_exported_model_does_not_shadow_the_settings_file() {
        let settings = Settings::parse(r#"{"model": "opus"}"#);
        let chosen = resolve_model(Some("   ".into()), &settings, |_| None);
        assert_eq!(chosen.as_deref(), Some("opus"));
    }

    /// The `env` block is variables, and is ranked like them: below what the build baked in. Only
    /// the top-level key is promoted above it.
    #[test]
    fn the_env_block_spelling_stays_below_the_baked_in_value() {
        let settings =
            Settings::parse(r#"{"env": {"BRAVE_AI_CHAT_DEFAULT_MODEL": "from-the-env-block"}}"#);
        let chosen = resolve_model(None, &settings, |_| Some("baked".into()));
        assert_eq!(chosen.as_deref(), Some("baked"));

        // And is still read where the build baked nothing in, so the spelling keeps working.
        let chosen = resolve_model(None, &settings, |_| None);
        assert_eq!(chosen.as_deref(), Some("from-the-env-block"));
    }

    /// With nothing said anywhere the default stands, which is what a fresh machine has.
    #[test]
    fn no_model_named_anywhere_leaves_the_default() {
        let chosen = resolve_model(None, &Settings::default(), |_| None);
        assert_eq!(chosen, None);
    }

    /// Where those three words come from they name a tier, and the tier's model is what the same
    /// file already said. Left as the word, the request reaches a backend that has never heard of
    /// it: Bedrock refuses an unknown model rather than substituting one.
    #[test]
    fn a_tier_alias_resolves_to_the_model_that_tier_names() {
        for (alias, expected) in [
            ("opus", "opus-arn"),
            ("sonnet", "sonnet-arn"),
            ("haiku", "haiku-arn"),
            // A hand-written file says `Opus` as readily as `opus`.
            ("Opus", "opus-arn"),
        ] {
            let config = Config::from_lookup(|key| match key {
                env_var::USE_BEDROCK => Some("1".into()),
                env_var::AWS_REGION => Some("us-west-2".into()),
                env_var::BEDROCK_OPUS_MODEL => Some("opus-arn".into()),
                env_var::BEDROCK_SONNET_MODEL => Some("sonnet-arn".into()),
                env_var::BEDROCK_HAIKU_MODEL => Some("haiku-arn".into()),
                env_var::DEFAULT_MODEL => Some(alias.into()),
                other => complete_env(other),
            })
            .unwrap();
            assert_eq!(config.default_model, expected, "{alias} did not resolve");
        }
    }

    /// Everything that is not one of the three words is used as written, which is what carries an
    /// inference-profile ARN and a name from the Brave roster alike.
    #[test]
    fn a_model_that_is_not_a_tier_alias_is_used_as_written() {
        for name in [
            "arn:aws:bedrock:us-west-2:1:application-inference-profile/abc",
            "claude-opus-4-8",
            "llama-3-8b-instruct",
        ] {
            let config = Config::from_lookup(|key| match key {
                env_var::USE_BEDROCK => Some("1".into()),
                env_var::AWS_REGION => Some("us-west-2".into()),
                env_var::BEDROCK_OPUS_MODEL => Some("opus-arn".into()),
                env_var::DEFAULT_MODEL => Some(name.into()),
                other => complete_env(other),
            })
            .unwrap();
            assert_eq!(config.default_model, name);
        }
    }

    /// Every route to a model resolves a name the same way, so a flag, a remembered choice and the
    /// settings key cannot accept different spellings of the same model.
    #[test]
    fn a_name_from_anywhere_resolves_as_the_settings_key_does() {
        let config = Config::from_lookup(|key| match key {
            env_var::USE_BEDROCK => Some("1".into()),
            env_var::AWS_REGION => Some("us-west-2".into()),
            env_var::BEDROCK_OPUS_MODEL => Some("opus-arn".into()),
            other => complete_env(other),
        })
        .unwrap();

        assert_eq!(config.model_named("opus"), "opus-arn");
        assert_eq!(
            config.model_named("sonnet"),
            bedrock::Tier::Sonnet.brave_model()
        );
        assert_eq!(config.model_named("automatic"), DEFAULT_MODEL);
        assert_eq!(
            config.model_named("llama-3-8b-instruct"),
            "llama-3-8b-instruct"
        );
    }

    /// Leo's automatic routing name is rewritten to brave-bot's own entry.
    #[test]
    fn the_legacy_automatic_name_becomes_the_brave_bot_default() {
        let config = Config::from_lookup(|key| match key {
            env_var::DEFAULT_MODEL => Some("automatic".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(config.default_model, DEFAULT_MODEL);
    }

    /// An ARN cannot be derived from a word, so a tier the AWS account left unset falls through to
    /// the roster every build can reach rather than being guessed at.
    #[test]
    fn an_alias_for_an_unconfigured_tier_falls_through_to_brave() {
        let config = Config::from_lookup(|key| match key {
            env_var::USE_BEDROCK => Some("1".into()),
            env_var::AWS_REGION => Some("us-west-2".into()),
            env_var::BEDROCK_OPUS_MODEL => Some("opus-arn".into()),
            env_var::DEFAULT_MODEL => Some("haiku".into()),
            other => complete_env(other),
        })
        .unwrap();
        assert_eq!(config.default_model, bedrock::Tier::Haiku.brave_model());
    }

    /// The case a settings file written for another tool actually lands in: no AWS account, so the
    /// word has to name something on the roster every build can reach. Left as the bare word it
    /// reaches an endpoint that has never heard of it and is silently reset, which is the key
    /// appearing to work and doing nothing.
    #[test]
    fn a_tier_alias_without_bedrock_resolves_against_the_brave_roster() {
        for (alias, tier) in [
            ("opus", bedrock::Tier::Opus),
            ("sonnet", bedrock::Tier::Sonnet),
            ("haiku", bedrock::Tier::Haiku),
        ] {
            let config = Config::from_lookup(|key| match key {
                env_var::DEFAULT_MODEL => Some(alias.into()),
                other => complete_env(other),
            })
            .unwrap();
            assert_eq!(config.default_model, tier.brave_model());
            // Never left as the bare word, which is the thing that silently did nothing.
            assert_ne!(config.default_model, alias);
        }
    }

    /// With no Brave credentials a Brave name reaches a service this build cannot sign for, so an
    /// unset tier resolves to the strongest one the AWS account did name instead.
    #[test]
    fn without_brave_credentials_an_unconfigured_tier_stays_on_aws() {
        let config = Config::from_lookup(|key| match key {
            env_var::USE_BEDROCK => Some("1".into()),
            env_var::AWS_REGION => Some("us-west-2".into()),
            env_var::BEDROCK_OPUS_MODEL => Some("opus-arn".into()),
            env_var::DEFAULT_MODEL => Some("haiku".into()),
            _ => None,
        })
        .unwrap();
        assert!(!config.serves_aichat());
        assert_eq!(config.default_model, "opus-arn");
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
