//! What `GET /v1/models` offers.
//!
//! Asked so a person can pick one, and for nothing else. No model reads any of this: the list
//! reaches a picker, the person there chooses, and what comes back is the name sent in the `model`
//! field of a later request. That field is routing, and the endorsement for it is the choice.
//!
//! The endpoint answers with a bare array rather than an OpenAI-style `{"data": [...]}` envelope,
//! and it lists concrete models only, so [`automatic`](bua_config::DEFAULT_MODEL) is added here.

use bua_config::Config;
use bua_core::event::Sink;
use bua_core::label::Label;
use bua_core::policy::Policy;
use bua_net::{Egress, Request};
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
}

impl Model {
    /// Let the server decide per request.
    ///
    /// Always offered: the endpoint lists concrete models and never this, and it is what an
    /// unrecognised name is reset to anyway, so it is the one choice that cannot fail to work.
    pub fn automatic() -> Self {
        Self {
            key: bua_config::DEFAULT_MODEL.to_string(),
            display_name: "Automatic".to_string(),
            premium: false,
        }
    }

    pub fn is_automatic(&self) -> bool {
        self.key == bua_config::DEFAULT_MODEL
    }
}

/// One entry as the server reports it.
///
/// Only the fields that matter to a choice are named. The endpoint reports vision, audio and video
/// support as well, none of which this agent uses.
#[derive(Debug, Deserialize)]
struct Listed {
    /// Absent for an entry the server has no name for, which cannot be requested and is dropped.
    key: Option<String>,
    display_name: String,
    /// A model that cannot call tools is no use here: every turn works by calling them, so
    /// choosing one would produce an agent unable to read or write anything.
    supports_tools: bool,
    options: ListedOptions,
}

#[derive(Debug, Deserialize)]
struct ListedOptions {
    /// `basic_and_premium` or `premium`.
    access: String,
}

/// The value of `access` that means a subscription is required.
const PREMIUM_ACCESS: &str = "premium";

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

/// The models worth offering, `automatic` first.
///
/// Separated from the request so the filtering is testable without a server.
fn usable(listed: Vec<Listed>) -> Vec<Model> {
    let mut models = vec![Model::automatic()];
    models.extend(
        listed
            .into_iter()
            .filter(|entry| entry.supports_tools)
            .filter_map(|entry| {
                Some(Model {
                    key: entry.key?,
                    display_name: entry.display_name,
                    premium: entry.options.access == PREMIUM_ACCESS,
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

    /// The shape the backend actually answers with: a bare array, and the name to send back in
    /// `key` rather than in `display_name`.
    #[test]
    fn a_bare_array_decodes_and_keeps_the_requestable_name() {
        let models = decoded(
            r#"[{"key":"claude-3-sonnet","display_name":"Claude 4 Sonnet","supports_tools":true,
                 "options":{"access":"premium"}}]"#,
        );
        assert_eq!(models.len(), 2, "{models:?}");
        assert_eq!(models[1].key, "claude-3-sonnet");
        assert_eq!(models[1].display_name, "Claude 4 Sonnet");
        assert!(models[1].premium);
    }

    #[test]
    fn a_free_model_is_not_premium() {
        let models = decoded(
            r#"[{"key":"llama-3-8b-instruct","display_name":"Llama 3 8B","supports_tools":true,
                 "options":{"access":"basic_and_premium"}}]"#,
        );
        assert!(!models[1].premium);
    }

    /// Every turn here works by calling tools, so a model that cannot call them would be an agent
    /// that can read and write nothing. Offering it would be offering a broken session.
    #[test]
    fn a_model_that_cannot_call_tools_is_not_offered() {
        let models = decoded(
            r#"[{"key":"mixtral-8x7b-instruct","display_name":"Mixtral 8x7B","supports_tools":false,
                 "options":{"access":"basic_and_premium"}}]"#,
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
            r#"[{"key":null,"display_name":"Nameless","supports_tools":true,
                 "options":{"access":"premium"}}]"#,
        );
        assert_eq!(models, vec![Model::automatic()], "{models:?}");
    }

    /// Added unconditionally, so a server that did list it must not produce two identical rows: a
    /// person cannot tell which of them they chose.
    #[test]
    fn automatic_is_offered_once_even_if_the_server_lists_it_too() {
        let models = decoded(
            r#"[{"key":"automatic","display_name":"Automatic","supports_tools":true,
                 "options":{"access":"basic_and_premium"}}]"#,
        );
        assert_eq!(models, vec![Model::automatic()], "{models:?}");
    }

    /// A field this agent does not use must not make an entry undecodable: the endpoint reports
    /// vision, audio and rate limits too, and it grows fields without asking.
    #[test]
    fn unknown_fields_do_not_break_decoding() {
        let models = decoded(
            r#"[{"key":"claude-3-sonnet","display_name":"Claude 4 Sonnet","supports_tools":true,
                 "vision_support":true,"is_near_model":false,
                 "options":{"access":"premium","description":"whatever","category":"chat"}}]"#,
        );
        assert_eq!(models.len(), 2, "{models:?}");
    }
}
