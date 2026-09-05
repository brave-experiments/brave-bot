//! What `GET /v1/models` offers.
//!
//! Asked so a person can pick one, and for nothing else. No model reads any of this: the list
//! reaches a picker, the person there chooses, and what comes back is the name sent in the `model`
//! field of a later request. That field is routing, and the endorsement for it is the choice.
//!
//! The endpoint answers with a bare array rather than an OpenAI-style `{"data": [...]}` envelope,
//! and it lists concrete models only, so [`automatic`](bravebot_config::DEFAULT_MODEL) is added here.

use bravebot_config::Config;
use bravebot_core::event::Sink;
use bravebot_core::label::Label;
use bravebot_core::policy::Policy;
use bravebot_net::{Egress, Request};
use serde::Deserialize;

use crate::ChatError;

/// A model the user may choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    /// What goes in a request's `model` field.
    pub key: String,
    /// What to show a person choosing.
    pub display_name: String,
    /// Whether a subscription is needed for it.
    pub premium: bool,
    /// Which service answers for it, where that is not Brave's own endpoint.
    ///
    /// What tells two rows offering the same slug apart, and what a picker files the row under.
    /// `None` is the Brave roster, whose name belongs to the layer that draws it: this field
    /// carries only what a configuration already called a service, never words composed here.
    pub provider: Option<String>,
    /// How large a request may get, in prompt tokens, as the endpoint advertises it.
    ///
    /// `None` for an entry that did not say, and for `automatic`, where the server picks the model
    /// per request and so no one figure describes it.
    ///
    /// Tokens, despite arriving in a field called
    /// `long_conversation_warning_character_limit`. The name is wrong at the source: the endpoint
    /// computes it as `conversation_token_limit * 0.8`, so it is a token count already discounted
    /// by a fifth, and reading it as characters would divide a budget that has already been made
    /// safe. Usable as a budget with no conversion.
    pub conversation_tokens: Option<u64>,
}

impl Model {
    /// Let the server decide per request.
    ///
    /// Always offered: the endpoint lists concrete models and never this, and it is what an
    /// unrecognised name is reset to anyway, so it is the one choice that cannot fail to work.
    pub fn automatic() -> Self {
        Self {
            key: bravebot_config::DEFAULT_MODEL.to_string(),
            display_name: "Automatic".to_string(),
            premium: false,
            // Brave's own endpoint resolves it per request, and that is the roster it belongs to.
            provider: None,
            // The server chooses per request, so there is no one model whose limit this could be.
            conversation_tokens: None,
        }
    }

    pub fn is_automatic(&self) -> bool {
        self.key == bravebot_config::DEFAULT_MODEL
    }
}

/// One entry as the server reports it.
///
/// Only the fields that matter to a choice, and the limit a budget is worked out from. The endpoint
/// reports a description and a maker too, neither of which decides anything here.
#[derive(Debug, Deserialize)]
struct Listed {
    /// Absent for an entry the server has no name for, which cannot be requested and is dropped.
    key: Option<String>,
    display_name: String,
    /// What the model can do: `chat`, `tools`, `vision`, `files` and others.
    ///
    /// Defaulted rather than required, so an entry omitting it is read as capable of nothing and
    /// dropped. A missing list is not a promise about tools.
    #[serde(default)]
    capabilities: Vec<String>,
    options: ListedOptions,
}

#[derive(Debug, Deserialize)]
struct ListedOptions {
    /// `basic_and_premium` or `premium`.
    access: String,
    /// The window the endpoint will work to, in **tokens**, whatever the name says.
    ///
    /// Computed at the source as `conversation_token_limit * 0.8`, so the fifth held back is
    /// already in it. Nothing is measured in characters here, and a client that divided it into
    /// tokens would compact five times sooner than it had to.
    ///
    /// The figure varies across the roster by a factor of thirty, which is why one constant cannot
    /// stand in for it. Optional, so an entry that stops reporting it falls back rather than
    /// failing to decode.
    #[serde(default)]
    long_conversation_warning_character_limit: Option<u64>,
}

/// The value of `access` that means a subscription is required.
const PREMIUM_ACCESS: &str = "premium";

/// The capability every turn here depends on.
///
/// Not everything the endpoint lists has it. The rest are chat-only models, and choosing one would
/// produce an agent that cannot read or write anything.
const TOOLS_CAPABILITY: &str = "tools";

/// Ask the endpoint what it offers, newest answer each time.
///
/// `automatic` comes first and is always present. The rest are whatever the server listed that can
/// call tools, in the order it listed them, since that order is the backend's own preference.
pub fn list<S: Sink>(
    policy: &mut Policy<'_, S>,
    config: &Config,
    egress: &Egress,
) -> Result<Vec<Model>, ChatError> {
    let request = Request::get(config.models_url()).header("accept", "application/json");

    // Through the egress gate like everything else, because that gate is the only way out to the
    // network. The label records where the bytes came from; nothing here branches on them beyond
    // the shape the endpoint documents.
    let response = egress.fetch(policy, request, Label::untrusted_public())?;
    let (bytes, _) = response.body.into_parts_for_decoding();

    let listed: Vec<Listed> = serde_json::from_slice(&bytes).map_err(|e| ChatError::Decode {
        detail: format!("{e} (received {} bytes from /v1/models)", bytes.len()),
    })?;

    Ok(usable(listed))
}

/// One entry of a gateway's own roster.
///
/// The OpenAI shape is `{"data": [{"id": ...}]}` and says nothing but the name. Everything else here
/// is a field gateways add: read where present, absent without complaint, because a gateway that
/// reports only what the shape requires still has a usable roster.
#[derive(Debug, Deserialize)]
struct ListedByGateway {
    /// What a request names. The only field the shape guarantees.
    id: String,
    /// The window, where the gateway reports one.
    #[serde(default)]
    context_length: Option<u64>,
    /// What the model accepts, which is where a gateway says whether it can call tools.
    ///
    /// Absent for a gateway that does not report it, and then nothing is filtered: a roster that
    /// says nothing about capabilities is not a roster claiming none, and dropping all of it would
    /// leave a person unable to pick anything.
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
}

/// Ask one gateway what models it serves.
///
/// The listing is content, and the person's pick off it is what endorses a request field: the same
/// footing the Brave roster arrives on, and the reason a fetched roster may reach a picker at all.
/// The destination is the gateway's own endpoint, which came from configuration rather than from
/// anything fetched.
///
/// Bearer-authenticated because some gateways refuse the listing otherwise, and because a roster is a
/// per-account fact wherever a gateway offers different models to different keys.
///
/// What this account may reach is asked for first, where the gateway answers such a question, and the
/// service-wide roster is the fallback. The narrower answer is the better one: a model the credential
/// cannot reach is a row that fails the moment somebody picks it, and the wide list is nearly three
/// times the size here. Only the fallback is guaranteed to exist, since the account-scoped route is a
/// gateway's own extension rather than part of the shape.
pub fn list_from_gateway<S: Sink>(
    policy: &mut Policy<'_, S>,
    provider: &bravebot_config::provider::Provider,
    token: &str,
    egress: &Egress,
) -> Result<Vec<Model>, ChatError> {
    match fetch_listing(policy, provider.account_models_url(), token, egress) {
        Ok(listed) => Ok(offered_by_gateway(provider, listed)),
        // Any failure falls through to the wide roster: a gateway with no such route answers 404,
        // one that has it under another name answers something undecodable, and neither is a reason
        // to offer nothing when a list that does work is one request away.
        Err(_) => {
            let listed = fetch_listing(policy, provider.models_url(), token, egress)?;
            Ok(offered_by_gateway(provider, listed))
        }
    }
}

/// One roster request, decoded.
fn fetch_listing<S: Sink>(
    policy: &mut Policy<'_, S>,
    url: String,
    token: &str,
    egress: &Egress,
) -> Result<Vec<ListedByGateway>, ChatError> {
    let request = Request::get(&url)
        .header("accept", "application/json")
        .header("authorization", format!("Bearer {token}"));

    let response = egress.fetch(policy, request, Label::untrusted_public())?;
    let (bytes, _) = response.body.into_parts_for_decoding();

    let listed: GatewayListing = serde_json::from_slice(&bytes).map_err(|e| ChatError::Decode {
        detail: format!("{e} (received {} bytes from {url})", bytes.len()),
    })?;
    Ok(listed.data)
}

/// The envelope a gateway's roster arrives in.
#[derive(Debug, Deserialize)]
struct GatewayListing {
    data: Vec<ListedByGateway>,
}

/// The picker rows one gateway's roster becomes, in the order it listed them.
///
/// Separated from the request so the filtering is testable without a server.
///
/// Every key is qualified by the provider's id, because that is what a choice is remembered as and
/// what later selects this gateway rather than another service offering the same name.
///
/// Nothing is capped. A gateway's roster runs to hundreds, and the picker filters as a person types,
/// so a limit here would be this program deciding somebody may not choose a model their gateway
/// serves.
fn offered_by_gateway(
    provider: &bravebot_config::provider::Provider,
    listed: Vec<ListedByGateway>,
) -> Vec<Model> {
    listed
        .into_iter()
        .filter(|entry| match entry.supported_parameters.as_ref() {
            Some(parameters) => parameters.iter().any(|it| it == TOOLS_CAPABILITY),
            None => true,
        })
        .filter(|entry| !entry.id.trim().is_empty())
        .map(|entry| Model {
            key: format!("{}/{}", provider.id, entry.id),
            // The bare name the gateway knows it by. What a person reads is composed by whoever
            // draws the picker, which is the layer that owns the words shown to somebody.
            display_name: entry.id.clone(),
            // A gateway is reached with the person's own bearer token, so a Leo subscription has
            // nothing to do with it.
            premium: false,
            // The name the block gave this service, which is what a person recognises it by and
            // what a configured model from the same gateway is filed under.
            provider: Some(provider.display_name().to_string()),
            // What the block said about this model outranks what the gateway reports, because a
            // stated window is somebody pinning a figure they know better than the roster does.
            // Failing that the reported one, and failing that the conservative default: a window
            // above the real one does not delay compaction, it removes it.
            conversation_tokens: Some(
                provider
                    .model(&entry.id)
                    .and_then(|model| model.context_window)
                    .or(entry.context_length)
                    .unwrap_or(bravebot_config::provider::CONTEXT_WINDOW),
            ),
        })
        .collect()
}

/// The models worth offering, `automatic` first.
///
/// Separated from the request so the filtering is testable without a server.
fn usable(listed: Vec<Listed>) -> Vec<Model> {
    let mut models = vec![Model::automatic()];
    models.extend(
        listed
            .into_iter()
            .filter(|entry| entry.capabilities.iter().any(|c| c == TOOLS_CAPABILITY))
            .filter_map(|entry| {
                Some(Model {
                    key: entry.key?,
                    display_name: entry.display_name,
                    premium: entry.options.access == PREMIUM_ACCESS,
                    provider: None,
                    conversation_tokens: entry
                        .options
                        .long_conversation_warning_character_limit
                        // The endpoint sends 1 where a model reports no window at all, and zero is
                        // not a budget either: both mean "it did not say" rather than "no room".
                        .filter(|limit| *limit > 1),
                })
            })
            // The endpoint has no reason to list one name twice, but the choice is a person's and
            // two identical rows would leave them unable to tell which they picked.
            .filter(|model| !model.is_automatic()),
    );
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(body: &str) -> Vec<Model> {
        usable(serde_json::from_str(body).expect("the test body parses"))
    }

    /// The shape the deployed endpoint answers with: a bare array, the name to send back in `key`
    /// rather than in `display_name`, and what it can do in `capabilities`.
    #[test]
    fn a_bare_array_decodes_and_keeps_the_requestable_name() {
        let models = decoded(
            r#"[{"key":"claude-3-sonnet","display_name":"Claude Sonnet",
                 "capabilities":["chat","tools","vision"],"options":{"access":"premium"}}]"#,
        );
        assert_eq!(models.len(), 2, "{models:?}");
        assert_eq!(models[1].key, "claude-3-sonnet");
        assert_eq!(models[1].display_name, "Claude Sonnet");
        assert!(models[1].premium);
    }

    #[test]
    fn a_free_model_is_not_premium() {
        let models = decoded(
            r#"[{"key":"claude-3-haiku","display_name":"Claude Haiku",
                 "capabilities":["chat","tools"],"options":{"access":"basic_and_premium"}}]"#,
        );
        assert!(!models[1].premium);
    }

    /// The window is the one thing here worth having that a person does not choose, and it arrives
    /// under a name that says characters while holding tokens.
    #[test]
    fn the_advertised_window_is_kept_as_tokens() {
        let models = decoded(
            r#"[{"key":"claude-opus","display_name":"Claude Opus","capabilities":["chat","tools"],
                 "options":{"access":"premium",
                            "long_conversation_warning_character_limit":102400}}]"#,
        );
        assert_eq!(models[1].conversation_tokens, Some(102_400));
    }

    /// An entry that says nothing about its window is not saying it has none. The caller falls back
    /// rather than compacting at whatever a missing field reads as.
    #[test]
    fn an_entry_that_advertises_no_window_reports_none() {
        let models = decoded(
            r#"[{"key":"mystery","display_name":"Mystery","capabilities":["chat","tools"],
                 "options":{"access":"premium"}}]"#,
        );
        assert_eq!(models[1].conversation_tokens, None);
    }

    /// What the endpoint sends for a model whose window it does not know. Taken as "did not say"
    /// rather than as a budget, which at one token would compact before a turn could start.
    #[test]
    fn the_placeholder_window_is_read_as_nothing_said() {
        let models = decoded(
            r#"[{"key":"mystery","display_name":"Mystery","capabilities":["chat","tools"],
                 "options":{"access":"premium",
                            "long_conversation_warning_character_limit":1}}]"#,
        );
        assert_eq!(models[1].conversation_tokens, None);
    }

    /// The server picks the model per request, so no one window describes it.
    #[test]
    fn automatic_advertises_no_window() {
        assert_eq!(Model::automatic().conversation_tokens, None);
    }

    /// Every turn here works by calling tools, so a chat-only model would be an agent that can read
    /// and write nothing. A good fraction of what the endpoint lists is chat-only, so this is the
    /// common case rather than a hypothetical one.
    #[test]
    fn a_model_that_cannot_call_tools_is_not_offered() {
        let models = decoded(
            r#"[{"key":"llama-3-8b-instruct","display_name":"Llama 3 8B",
                 "capabilities":["chat","files"],"options":{"access":"basic_and_premium"}}]"#,
        );
        assert_eq!(models, vec![Model::automatic()], "{models:?}");
    }

    /// An entry claiming nothing is not claiming tools. Read as capable of nothing rather than
    /// waved through, since the whole point of the check is that a chat-only model cannot work.
    #[test]
    fn an_entry_with_no_capabilities_is_not_offered() {
        let models = decoded(
            r#"[{"key":"mystery","display_name":"Mystery","options":{"access":"premium"}}]"#,
        );
        assert_eq!(models, vec![Model::automatic()], "{models:?}");
    }

    /// The endpoint lists concrete models and never this one, so it has to be added or there would
    /// be no way back to letting the server choose.
    #[test]
    fn automatic_is_offered_first_even_when_the_server_lists_nothing() {
        let models = decoded("[]");
        assert_eq!(models, vec![Model::automatic()]);
        assert!(models[0].is_automatic());
    }

    /// An entry with no key cannot be put in a request's `model` field, so it is not a choice.
    #[test]
    fn an_entry_without_a_key_is_dropped() {
        let models = decoded(
            r#"[{"key":null,"display_name":"Nameless","capabilities":["chat","tools"],
                 "options":{"access":"premium"}}]"#,
        );
        assert_eq!(models, vec![Model::automatic()], "{models:?}");
    }

    /// Added unconditionally, so a server that did list it must not produce two identical rows: a
    /// person cannot tell which of them they chose.
    #[test]
    fn automatic_is_offered_once_even_if_the_server_lists_it_too() {
        let models = decoded(
            r#"[{"key":"automatic","display_name":"Automatic","capabilities":["chat","tools"],
                 "options":{"access":"basic_and_premium"}}]"#,
        );
        assert_eq!(models, vec![Model::automatic()], "{models:?}");
    }

    /// A field this agent does not use must not make an entry undecodable. The endpoint grows
    /// fields without asking: it reported `supports_tools` once and reports `capabilities` now.
    #[test]
    fn unknown_fields_do_not_break_decoding() {
        let models = decoded(
            r#"[{"key":"claude-3-sonnet","display_name":"Claude Sonnet",
                 "capabilities":["chat","tools"],"is_near_model":false,"is_suggested_model":true,
                 "options":{"access":"premium","description":"whatever","display_maker":"Anthropic",
                            "max_associated_content_length":16000}}]"#,
        );
        assert_eq!(models.len(), 2, "{models:?}");
    }

    /// The shape of a real reply, field for field, including the ones this ignores: the array is
    /// bare, tool support lives in `capabilities`, and access lives under `options`.
    ///
    /// The shape is what is pinned, not the roster. This decoded `supports_tools` until the
    /// deployed endpoint was actually asked, and every listing would have failed; the fixture
    /// exists so the next such change fails here rather than at a user's prompt. The entries are
    /// stand-ins deliberately, since a snapshot of the live roster in a public repository would
    /// publish which models are deployed and go stale besides.
    #[test]
    fn a_real_response_shape_decodes() {
        let models = decoded(include_str!("../tests/models_response.json"));

        assert!(models[0].is_automatic(), "automatic was not offered first");
        let keys: Vec<&str> = models.iter().map(|m| m.key.as_str()).collect();
        assert!(keys.contains(&"claude-3-sonnet"), "{keys:?}");
        assert!(
            !keys.contains(&"llama-3-8b-instruct"),
            "a chat-only model was offered: {keys:?}"
        );

        let sonnet = models
            .iter()
            .find(|m| m.key == "claude-3-sonnet")
            .expect("the premium entry survived");
        assert_eq!(sonnet.display_name, "Claude Sonnet");
        assert!(sonnet.premium);
    }

    /// A gateway configured with no models at all, which is the case its roster is fetched for.
    fn gateway() -> bravebot_config::provider::Provider {
        let serde_json::Value::Object(root) = serde_json::from_str(
            r#"{"provider": {"openrouter": {
                "options": {"baseURL": "https://openrouter.example.invalid/api/v1"}
            }}}"#,
        )
        .expect("json") else {
            panic!("not an object");
        };
        bravebot_config::provider::Provider::all(&root)
            .pop()
            .expect("one provider")
    }

    fn from_gateway(provider: &bravebot_config::provider::Provider, body: &str) -> Vec<Model> {
        let listed: GatewayListing = serde_json::from_str(body).expect("the test body parses");
        offered_by_gateway(provider, listed.data)
    }

    /// The shape a gateway answers with: an envelope around entries whose only certain field is the
    /// name. Every key is qualified by the provider's id, since that is what selects this service
    /// later and a fetched roster is not written down anywhere that could say so instead.
    #[test]
    fn a_gateway_roster_is_offered_under_names_that_say_which_gateway_serves_them() {
        let provider = gateway();
        let models = from_gateway(
            &provider,
            r#"{"data": [{"id": "z-ai/glm-4.6"}, {"id": "moonshot/kimi-k2"}]}"#,
        );
        let keys: Vec<&str> = models.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            ["openrouter/z-ai/glm-4.6", "openrouter/moonshot/kimi-k2"]
        );
        // The bare name, for whoever composes the row a person reads.
        assert_eq!(models[0].display_name, "z-ai/glm-4.6");
    }

    /// The same slug is reachable through more than one service, billed and credentialled
    /// differently. A fetched row is the one place nothing written down could say which is which,
    /// so the gateway it came from travels with it.
    #[test]
    fn a_fetched_row_carries_the_gateway_that_serves_it() {
        let models = from_gateway(&gateway(), r#"{"data": [{"id": "z-ai/glm-4.6"}]}"#);
        assert_eq!(models[0].provider.as_deref(), Some("openrouter"));
        assert_eq!(
            Model::automatic().provider,
            None,
            "the Brave roster names no service of its own"
        );
    }

    /// A window the gateway reports is worth having: it is the one fact about a fetched model that
    /// nobody can type, and the default is deliberately far below what most of them offer.
    #[test]
    fn a_window_a_gateway_reports_is_taken_from_the_listing() {
        let models = from_gateway(
            &gateway(),
            r#"{"data": [{"id": "z-ai/glm-4.6", "context_length": 262144}]}"#,
        );
        assert_eq!(models[0].conversation_tokens, Some(262_144));
    }

    /// A gateway saying nothing about a window is not saying there is none. The conservative figure
    /// stands in, because a budget above the real window does not delay compaction but removes it.
    #[test]
    fn a_fetched_model_with_no_reported_window_gets_the_assumed_one() {
        let models = from_gateway(&gateway(), r#"{"data": [{"id": "z-ai/glm-4.6"}]}"#);
        assert_eq!(
            models[0].conversation_tokens,
            Some(bravebot_config::provider::CONTEXT_WINDOW)
        );
    }

    /// Where the block pinned a window for a model, that figure is the person's own and outranks
    /// what the roster reports, which is the whole reason stating one stays possible.
    #[test]
    fn a_window_the_block_stated_outranks_the_one_reported() {
        let serde_json::Value::Object(root) = serde_json::from_str(
            r#"{"provider": {"openrouter": {
                "options": {"baseURL": "https://openrouter.example.invalid/api/v1"},
                "models": {"z-ai/glm-4.6": {"limit": {"context": 32000, "output": 8192}}}
            }}}"#,
        )
        .expect("json") else {
            panic!("not an object");
        };
        let provider = bravebot_config::provider::Provider::all(&root)
            .pop()
            .expect("one provider");
        let models = from_gateway(
            &provider,
            r#"{"data": [{"id": "z-ai/glm-4.6", "context_length": 262144}]}"#,
        );
        assert_eq!(models[0].conversation_tokens, Some(32_000));
    }

    /// Every turn here calls tools, so a chat-only model is a choice that produces an agent which
    /// cannot read or write anything. Dropped on the same grounds as on the Brave roster.
    #[test]
    fn a_gateway_model_that_cannot_call_tools_is_not_offered() {
        let models = from_gateway(
            &gateway(),
            r#"{"data": [
                {"id": "can/talk", "supported_parameters": ["temperature"]},
                {"id": "can/work", "supported_parameters": ["temperature", "tools"]}
            ]}"#,
        );
        let keys: Vec<&str> = models.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["openrouter/can/work"]);
    }

    /// A gateway that reports no capabilities at all is not claiming its models have none. Filtering
    /// on a field the shape does not require would empty the picker for every gateway but one.
    #[test]
    fn a_gateway_that_reports_no_capabilities_still_offers_its_models() {
        let models = from_gateway(&gateway(), r#"{"data": [{"id": "z-ai/glm-4.6"}]}"#);
        assert_eq!(models.len(), 1);
    }

    /// A name is what a request carries, so an entry without one cannot be asked for. Nothing else
    /// on the entry could stand in for it.
    #[test]
    fn a_fetched_entry_with_no_usable_name_is_dropped() {
        let models = from_gateway(
            &gateway(),
            r#"{"data": [{"id": "  "}, {"id": "real/one"}]}"#,
        );
        let keys: Vec<&str> = models.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["openrouter/real/one"]);
    }

    /// A gateway is reached with somebody's own bearer token, so a Leo subscription has nothing to
    /// do with it. Marked premium, every row would ask them to import one to use what they pay for.
    #[test]
    fn fetched_gateway_models_are_not_marked_premium() {
        let models = from_gateway(&gateway(), r#"{"data": [{"id": "z-ai/glm-4.6"}]}"#);
        assert!(!models[0].premium);
    }
}
