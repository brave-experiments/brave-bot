//! Configuration for reaching an OpenAI-compatible gateway.
//!
//! The `provider` block in `~/.bravebot/settings.json`, in opencode's shape, so a block copied out
//! of `opencode.json` configures this agent unedited. That constraint decides most of what follows:
//! nothing is required that opencode does not require, no field is added however useful it would be,
//! and a field this crate does not know is read past rather than refused.
//!
//! What this does not hold is a resolved credential. A token is named, by the variables in `env` or
//! by `options.apiKey`, and read when a request needs signing.

/// The context window a gateway model is assumed to have, in prompt tokens.
///
/// `limit.context` is optional, because opencode does not require it and a window is a property of
/// the model and the upstream serving it rather than of the person writing the file. Asked for, it
/// would be guessed at, and a guess in the file looks authoritative where a default does not.
///
/// Deliberately the same conservative figure Bedrock assumes, and for the same reason: being wrong
/// upward does not make compaction late, it removes it. [`crate::env_var::CONTEXT_BUDGET`] overrides
/// it for anyone who knows better.
pub const CONTEXT_WINDOW: u64 = crate::bedrock::CONTEXT_WINDOW;

/// One model a gateway offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    /// What the gateway calls it, which is what a request names.
    pub id: String,
    /// The context window, where the file stated one.
    ///
    /// `None` reads as [`CONTEXT_WINDOW`]. Kept distinct from a stated figure so that
    /// `doctor` can say which models are running on the default.
    pub context_window: Option<u64>,
    /// Whatever `options` held, merged into the request body and never interpreted.
    ///
    /// Opaque on purpose: a gateway's routing controls are its own invention, and a schema
    /// enumerating them is a schema that changes when the gateway adds a field. Trusted exactly as
    /// far as a variable the person exported would be, and no model output can reach it.
    pub options: Option<serde_json::Map<String, serde_json::Value>>,
}

impl Model {
    /// The window to budget against, stated or assumed.
    pub fn window(&self) -> u64 {
        self.context_window.unwrap_or(CONTEXT_WINDOW)
    }
}

/// One gateway, and the models it was configured to offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    /// The key this provider had in the block, which is what a picker row names it by.
    ///
    /// A term that cannot collide with a name a service chose for itself, which is what
    /// distinguishing two services offering the same model requires.
    pub id: String,
    /// What to show a person choosing, where the block said something friendlier than the id.
    pub name: Option<String>,
    /// Where to send requests, without a trailing slash.
    pub base_url: String,
    /// Variables that may hold the bearer token, in the order to try them.
    pub env: Vec<String>,
    /// A token written into the file directly.
    ///
    /// Supported because it is opencode's field, not because it is a good idea: a long-lived
    /// credential in a file is a credential in a file people paste into issues. Naming a variable in
    /// `env` keeps it wherever the person already keeps secrets.
    pub api_key: Option<String>,
    /// The models this provider offers, in the order the file listed them.
    ///
    /// Possibly empty, because opencode does not require `models`. A provider offering nothing is
    /// reported as such rather than guessed at: a gateway roster is too large and too fluid to
    /// enumerate, so there is nothing to fall back to.
    pub models: Vec<Model>,
}

impl Provider {
    /// Every provider in a settings root, in the order the file listed them.
    ///
    /// Empty where the block is absent or shaped differently, on the same footing as the rest of the
    /// file: a half-typed settings file must not stop a session.
    ///
    /// A provider with no `baseURL` is dropped. Unlike a missing window there is nothing to assume:
    /// with no host there is no request to build, and inventing one produces failures far from the
    /// mistake.
    pub fn all(root: &serde_json::Map<String, serde_json::Value>) -> Vec<Self> {
        let Some(serde_json::Value::Object(block)) = root.get("provider") else {
            return Vec::new();
        };
        block
            .iter()
            .filter_map(|(id, entry)| match entry {
                serde_json::Value::Object(entry) => Self::one(id, entry),
                _ => None,
            })
            .collect()
    }

    /// One entry under `provider`, or `None` where it does not describe a reachable service.
    fn one(id: &str, entry: &serde_json::Map<String, serde_json::Value>) -> Option<Self> {
        let options = match entry.get("options") {
            Some(serde_json::Value::Object(options)) => Some(options),
            _ => None,
        };
        let base_url = options
            .and_then(|options| options.get("baseURL"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty())?;

        Some(Self {
            id: id.to_string(),
            name: string(entry.get("name")),
            base_url: base_url.trim_end_matches('/').to_string(),
            env: names(entry.get("env")),
            api_key: options.and_then(|options| string(options.get("apiKey"))),
            models: models(entry.get("models")),
        })
    }

    /// The URL one chat completion goes to.
    pub fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    /// What to show a person choosing this provider.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// The model this provider offers under `id`, if it offers one.
    ///
    /// Used to check a remembered choice before it becomes a request, and to find the window a
    /// budget is taken from.
    pub fn model(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|model| model.id == id)
    }

    /// Whether a name is one of this provider's models.
    pub fn offers(&self, model: &str) -> bool {
        self.model(model).is_some()
    }

    /// The bearer token, from the first variable that holds one or from the file.
    ///
    /// A variable first, so that a file naming one does not have the value read out from under it by
    /// a stale `apiKey`. Absent where nothing holds a token: that is a request this configuration
    /// cannot sign, and the caller reports it rather than sending an unauthenticated one.
    pub fn token(&self, lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
        self.env
            .iter()
            .filter_map(|name| lookup(name))
            .map(|value| value.trim().to_string())
            .find(|value| !value.is_empty())
            .or_else(|| self.api_key.clone())
    }
}

/// A string value with surrounding space removed, or `None` when nothing is left.
fn string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// The `env` array: variable names that may hold the token.
fn names(value: Option<&serde_json::Value>) -> Vec<String> {
    let Some(serde_json::Value::Array(names)) = value else {
        return Vec::new();
    };
    names.iter().filter_map(|name| string(Some(name))).collect()
}

/// The `models` block, in the order the file listed it.
fn models(value: Option<&serde_json::Value>) -> Vec<Model> {
    let Some(serde_json::Value::Object(block)) = value else {
        return Vec::new();
    };
    block
        .iter()
        .map(|(id, entry)| {
            let entry = entry.as_object();
            Model {
                id: id.to_string(),
                context_window: entry.and_then(|entry| window(entry.get("limit"))),
                options: entry.and_then(|entry| match entry.get("options") {
                    Some(serde_json::Value::Object(options)) if !options.is_empty() => {
                        Some(options.clone())
                    }
                    _ => None,
                }),
            }
        })
        .collect()
}

/// The window a `limit` block states, where it states a usable one.
///
/// opencode requires `context` and `output` together once `limit` is present, so a block naming only
/// one of them is not a `limit` and its figure is not read. Honoured because a copied block that
/// opencode rejects should not be read here as though it said something.
fn window(limit: Option<&serde_json::Value>) -> Option<u64> {
    let serde_json::Value::Object(limit) = limit? else {
        return None;
    };
    let context = limit.get("context")?.as_u64()?;
    limit.get("output")?.as_u64()?;
    (context > 0).then_some(context)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Vec<Provider> {
        let serde_json::Value::Object(root) = serde_json::from_str(text).expect("json") else {
            panic!("not an object");
        };
        Provider::all(&root)
    }

    fn one(text: &str) -> Provider {
        let mut all = parsed(text);
        assert_eq!(all.len(), 1, "expected exactly one provider");
        all.remove(0)
    }

    /// The point of the block: a `provider` entry copied out of `opencode.json` configures this
    /// agent without being rewritten first.
    #[test]
    fn a_provider_block_is_read() {
        let provider = one(r#"{"provider": {"openrouter": {
                "name": "OpenRouter",
                "env": ["OPENROUTER_API_KEY"],
                "options": {"baseURL": "https://openrouter.ai/api/v1"},
                "models": {"z-ai/glm-4.6": {}}
            }}}"#);
        assert_eq!(provider.id, "openrouter");
        assert_eq!(provider.display_name(), "OpenRouter");
        assert_eq!(provider.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(provider.env, ["OPENROUTER_API_KEY"]);
        assert!(provider.offers("z-ai/glm-4.6"));
    }

    /// opencode requires nothing of a model entry, so neither may this. An empty entry is a legal
    /// opencode model and has to stay a legal one here, or a copied block stops working.
    #[test]
    fn a_model_entry_may_be_empty() {
        let provider = one(r#"{"provider": {"gw": {
                "options": {"baseURL": "https://example.invalid/v1"},
                "models": {"some/model": {}}
            }}}"#);
        let model = provider.model("some/model").expect("offered");
        assert_eq!(model.context_window, None);
        assert_eq!(model.options, None);
    }

    /// A window nobody stated is the conservative default rather than a refusal. Requiring the
    /// number asks for one the person does not have, and a guess typed to satisfy a requirement
    /// looks authoritative where a default does not.
    #[test]
    fn a_model_without_a_stated_window_gets_the_assumed_one() {
        let provider = one(r#"{"provider": {"gw": {
                "options": {"baseURL": "https://example.invalid/v1"},
                "models": {"some/model": {}}
            }}}"#);
        assert_eq!(
            provider.model("some/model").expect("offered").window(),
            CONTEXT_WINDOW
        );
    }

    /// The case `limit` exists for: an upstream pinned to a smaller window than the model's own.
    /// A default above the real window would not delay compaction, it would remove it.
    #[test]
    fn a_stated_window_is_read() {
        let provider = one(r#"{"provider": {"gw": {
                "options": {"baseURL": "https://example.invalid/v1"},
                "models": {"anthropic/claude-sonnet-4.5": {"limit": {"context": 200000, "output": 64000}}}
            }}}"#);
        let model = provider
            .model("anthropic/claude-sonnet-4.5")
            .expect("offered");
        assert_eq!(model.context_window, Some(200_000));
        assert_eq!(model.window(), 200_000);
    }

    /// opencode requires `context` and `output` together once `limit` is present. A block naming
    /// only one of them is one opencode rejects, so reading its figure here would honour a shape
    /// the block is supposed to share.
    #[test]
    fn a_limit_missing_either_half_states_no_window() {
        for text in [
            r#"{"provider": {"gw": {"options": {"baseURL": "https://e.invalid/v1"},
                "models": {"m": {"limit": {"context": 200000}}}}}}"#,
            r#"{"provider": {"gw": {"options": {"baseURL": "https://e.invalid/v1"},
                "models": {"m": {"limit": {"output": 64000}}}}}}"#,
            r#"{"provider": {"gw": {"options": {"baseURL": "https://e.invalid/v1"},
                "models": {"m": {"limit": {"context": 0, "output": 64000}}}}}}"#,
            r#"{"provider": {"gw": {"options": {"baseURL": "https://e.invalid/v1"},
                "models": {"m": {"limit": "wide"}}}}}"#,
        ] {
            let provider = one(text);
            assert_eq!(
                provider.model("m").expect("offered").context_window,
                None,
                "{text:?} stated a window"
            );
        }
    }

    /// Reading past a field is what makes a copied block work. opencode's own fields mean nothing
    /// here, and refusing them would defeat the reason the shape was borrowed.
    #[test]
    fn fields_this_crate_does_not_know_are_read_past() {
        let provider = one(r#"{"provider": {"gw": {
                "npm": "@ai-sdk/openai-compatible",
                "api": "https://example.invalid",
                "options": {"baseURL": "https://example.invalid/v1", "timeout": 30000},
                "models": {"some/model": {
                    "name": "Some Model",
                    "family": "glm",
                    "release_date": "2026-01-01",
                    "cost": {"input": 0.1, "output": 0.2},
                    "modalities": {"input": ["text"], "output": ["text"]},
                    "limit": {"context": 131072, "output": 8192}
                }}
            }}}"#);
        let model = provider.model("some/model").expect("offered");
        assert_eq!(model.context_window, Some(131_072));
    }

    /// `options` reaches the request body whole and is never parsed. A gateway's routing controls
    /// are its own invention, so a schema enumerating them is one that changes when it adds a field.
    #[test]
    fn model_options_are_carried_without_being_interpreted() {
        let provider = one(r#"{"provider": {"gw": {
                "options": {"baseURL": "https://example.invalid/v1"},
                "models": {"m": {"options": {"provider": {"order": ["amazon-bedrock"], "allow_fallbacks": false}}}}
            }}}"#);
        let options = provider
            .model("m")
            .expect("offered")
            .options
            .clone()
            .expect("options");
        assert_eq!(
            options.get("provider").and_then(|it| it.get("order")),
            Some(&serde_json::json!(["amazon-bedrock"]))
        );
    }

    /// With no host there is no request to build, and inventing one produces a failure far from the
    /// mistake. Unlike a window, there is nothing conservative to assume.
    #[test]
    fn a_provider_without_a_base_url_is_not_offered() {
        for text in [
            r#"{"provider": {"gw": {"models": {"m": {}}}}}"#,
            r#"{"provider": {"gw": {"options": {}}}}"#,
            r#"{"provider": {"gw": {"options": {"baseURL": ""}}}}"#,
            r#"{"provider": {"gw": {"options": {"baseURL": "   "}}}}"#,
            r#"{"provider": {"gw": {"options": {"baseURL": 1}}}}"#,
            r#"{"provider": {"gw": {"options": "not a block"}}}"#,
        ] {
            assert!(parsed(text).is_empty(), "{text:?} was offered");
        }
    }

    /// A trailing slash on the endpoint would otherwise produce a double slash in the path, which
    /// some gateways route differently and others reject.
    #[test]
    fn a_trailing_slash_on_the_endpoint_is_dropped() {
        let provider =
            one(r#"{"provider": {"gw": {"options": {"baseURL": "https://example.invalid/v1/"}}}}"#);
        assert_eq!(
            provider.chat_completions_url(),
            "https://example.invalid/v1/chat/completions"
        );
    }

    /// A variable is preferred to a value in the file, so that naming one does not have the value
    /// read out from under it by an `apiKey` left behind.
    #[test]
    fn a_named_variable_holds_the_token_before_the_file_does() {
        let provider = one(r#"{"provider": {"gw": {
                "env": ["ABSENT_ONE", "PRESENT_ONE"],
                "options": {"baseURL": "https://example.invalid/v1", "apiKey": "in-the-file"}
            }}}"#);
        let token = provider.token(|name| match name {
            "PRESENT_ONE" => Some("from-the-environment".to_string()),
            _ => None,
        });
        assert_eq!(token.as_deref(), Some("from-the-environment"));
    }

    /// Supported because it is opencode's field. A copied block that authenticates this way has to
    /// keep working, whatever this file recommends instead.
    #[test]
    fn a_token_written_into_the_file_is_still_read() {
        let provider = one(r#"{"provider": {"gw": {
                "options": {"baseURL": "https://example.invalid/v1", "apiKey": "in-the-file"}
            }}}"#);
        assert_eq!(provider.token(|_| None).as_deref(), Some("in-the-file"));
    }

    /// A request this configuration cannot sign is reported rather than sent unauthenticated.
    #[test]
    fn a_provider_with_nothing_holding_a_token_has_none() {
        let provider = one(r#"{"provider": {"gw": {
                "env": ["ABSENT_ONE"],
                "options": {"baseURL": "https://example.invalid/v1"}
            }}}"#);
        assert_eq!(provider.token(|_| None), None);
        assert_eq!(provider.token(|_| Some("   ".to_string())), None);
    }

    /// Two gateways are ordinary. The block being a map is what makes a duplicate id
    /// unrepresentable, rather than a rule something has to remember to enforce.
    #[test]
    fn more_than_one_provider_may_be_configured() {
        let all = parsed(
            r#"{"provider": {
                "openrouter": {"options": {"baseURL": "https://openrouter.ai/api/v1"}},
                "together": {"options": {"baseURL": "https://api.together.xyz/v1"}}
            }}"#,
        );
        let ids: Vec<&str> = all.iter().map(|it| it.id.as_str()).collect();
        assert_eq!(ids, ["openrouter", "together"]);
    }

    /// Where the block said nothing friendlier, the id is what a picker row names it by: a term
    /// that cannot collide with a name a service chose for itself.
    #[test]
    fn a_provider_with_no_name_is_shown_by_its_id() {
        let provider = one(
            r#"{"provider": {"openrouter": {"options": {"baseURL": "https://e.invalid/v1"}}}}"#,
        );
        assert_eq!(provider.display_name(), "openrouter");
    }

    /// A provider that turns nothing on is still a provider, and "no models configured" is a better
    /// thing to report than a guessed roster. A gateway's is too large and too fluid to enumerate.
    #[test]
    fn a_provider_may_offer_no_models() {
        let provider =
            one(r#"{"provider": {"gw": {"options": {"baseURL": "https://e.invalid/v1"}}}}"#);
        assert!(provider.models.is_empty());
        assert!(!provider.offers("anything"));
    }

    /// Every failure reads as absence, on the same footing as the `env` block: a half-typed
    /// settings file must not stop a session.
    #[test]
    fn a_malformed_provider_block_configures_nothing() {
        for text in [
            r#"{}"#,
            r#"{"provider": {}}"#,
            r#"{"provider": "not a block"}"#,
            r#"{"provider": []}"#,
            r#"{"provider": {"gw": "not a block"}}"#,
            r#"{"provider": {"gw": []}}"#,
            r#"{"providers": {"gw": {"options": {"baseURL": "https://e.invalid/v1"}}}}"#,
        ] {
            assert!(parsed(text).is_empty(), "{text:?} configured something");
        }
    }
}
