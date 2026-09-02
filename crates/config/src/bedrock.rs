//! Configuration for reaching Claude through AWS Bedrock.
//!
//! Present or absent, never half configured. [`Bedrock::from_lookup`] answers `None` unless the
//! switch is on and a region is named, so a caller holding one of these knows where to send a
//! request without checking anything further.
//!
//! What this does not hold is a credential. Bedrock authenticates with a SigV4 signature over
//! short-lived keys, and those expire during a session, so they are fetched when a request needs
//! them rather than read once at startup.

use crate::env_var;

/// Which tier a model name is for.
///
/// Three because that is what the settings block names. A tier is not a capability claim: it is
/// which of the three variables supplied the name, and the picker shows the ones that were set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Opus,
    Sonnet,
    Haiku,
}

impl Tier {
    /// Every tier, strongest first, which is the order a picker offers them in.
    pub const ALL: [Tier; 3] = [Tier::Opus, Tier::Sonnet, Tier::Haiku];

    /// The variable naming this tier's model.
    pub fn env_var(self) -> &'static str {
        match self {
            Tier::Opus => env_var::BEDROCK_OPUS_MODEL,
            Tier::Sonnet => env_var::BEDROCK_SONNET_MODEL,
            Tier::Haiku => env_var::BEDROCK_HAIKU_MODEL,
        }
    }

    /// What to show a person choosing.
    pub fn display_name(self) -> &'static str {
        match self {
            Tier::Opus => "Opus",
            Tier::Sonnet => "Sonnet",
            Tier::Haiku => "Haiku",
        }
    }
}

/// Everything needed to talk to Bedrock, when a build is pointed at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bedrock {
    /// Which region to sign for and send to.
    pub region: String,
    /// Which AWS profile to resolve credentials from, when one was named.
    pub profile: Option<String>,
    /// The model each configured tier names, strongest first.
    ///
    /// Possibly empty: a block that turns Bedrock on without naming a model is still Bedrock, and
    /// the resulting "no models configured" is a better thing to report than a guessed ARN.
    models: Vec<(Tier, String)>,
}

/// The context window a Bedrock model is assumed to have, in prompt tokens.
///
/// Bedrock has no endpoint that reports a window, and an inference-profile ARN does not say which
/// model it resolves to, so unlike the aichat backend there is nothing to ask. This figure is what
/// an opaque profile ARN actually gets: the larger windows require the served model to be
/// recognised by name, and an ARN that cannot be resolved to one does not qualify.
///
/// Deliberately not the advertised window of any particular model. Being wrong upward does not make
/// compaction late, it removes it, so a figure that holds for the unrecognised case is the safer of
/// the two errors. [`env_var::CONTEXT_BUDGET`] overrides it for anyone who knows better.
pub const CONTEXT_WINDOW: u64 = 131_072;

// Compaction depends on the budget sitting under the window: above it, compaction is not delayed but
// removed, silently. A compile is a better place to find that out than a transcript.
const _: () = assert!(CONTEXT_WINDOW > crate::DEFAULT_CONTEXT_BUDGET);

impl Bedrock {
    /// Read Bedrock configuration from a lookup, or `None` if this build is not pointed at it.
    ///
    /// The switch alone is not enough. Without a region there is nothing to sign for and no host to
    /// send to, so a block that sets the switch and stops is treated as not configured: falling
    /// back to the aichat backend leaves a working agent, whereas guessing a region produces
    /// requests that fail somewhere far from the mistake.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let enabled = lookup(env_var::USE_BEDROCK)
            .map(|value| value.trim() == "1")
            .unwrap_or(false);
        if !enabled {
            return None;
        }

        let region = trimmed(lookup(env_var::AWS_REGION))?;
        let profile = trimmed(lookup(env_var::AWS_PROFILE));

        let models = Tier::ALL
            .into_iter()
            .filter_map(|tier| trimmed(lookup(tier.env_var())).map(|name| (tier, name)))
            .collect();

        Some(Self {
            region,
            profile,
            models,
        })
    }

    /// The URL for one request against a model, streamed or not.
    ///
    /// The model name is percent-encoded because an inference-profile ARN contains colons and
    /// slashes, and pasting one into a path unencoded produces a URL whose path segments are not
    /// the ones intended.
    pub fn invoke_url(&self, model: &str, streaming: bool) -> String {
        let route = if streaming {
            "invoke-with-response-stream"
        } else {
            "invoke"
        };
        format!(
            "https://{}/model/{}/{route}",
            self.host(),
            encode_path_segment(model)
        )
    }

    /// The host requests go to, which is also what SigV4 signs.
    pub fn host(&self) -> String {
        format!("bedrock-runtime.{}.amazonaws.com", self.region)
    }

    /// Every configured tier and the model it names, strongest first.
    pub fn models(&self) -> &[(Tier, String)] {
        &self.models
    }

    /// The model a tier names, if that tier was configured.
    pub fn model_for(&self, tier: Tier) -> Option<&str> {
        self.models
            .iter()
            .find(|(configured, _)| *configured == tier)
            .map(|(_, name)| name.as_str())
    }

    /// The model to use when the person has not chosen one.
    ///
    /// The strongest configured tier. A default that reached for the cheapest would quietly answer
    /// a hard question with the weakest model available, and the person who configured three tiers
    /// asked for the best of them by naming it.
    pub fn default_model(&self) -> Option<&str> {
        self.models.first().map(|(_, name)| name.as_str())
    }

    /// Whether a name is one of the configured models.
    ///
    /// Used to check a remembered choice before it becomes a request. Bedrock rejects an unknown
    /// model rather than substituting one, so a stale name from `~/.bravebot/model` has to be
    /// noticed here rather than sent.
    pub fn offers(&self, model: &str) -> bool {
        self.models.iter().any(|(_, name)| name == model)
    }
}

/// A value with surrounding space removed, or `None` when nothing is left.
///
/// A blank is absence: a placeholder settings file leaves values empty, and reading `""` as a
/// region would build a request for a host that is just a dot.
fn trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Percent-encode one path segment.
///
/// Written out rather than taken from a dependency: what needs escaping in a path segment is a
/// short fixed set, and an ARN only contains a few of them. Everything not explicitly unreserved is
/// escaped, so this errs toward encoding rather than guessing which bytes a service tolerates.
fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(pairs: &[(&str, &str)]) -> Option<Bedrock> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Bedrock::from_lookup(|name| {
            owned
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        })
    }

    /// The common case: the switch, a region, and the three tiers.
    #[test]
    fn a_complete_block_configures_bedrock() {
        let bedrock = configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::AWS_PROFILE, "some-profile"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
            (env_var::BEDROCK_SONNET_MODEL, "sonnet-arn"),
            (env_var::BEDROCK_HAIKU_MODEL, "haiku-arn"),
        ])
        .expect("configured");
        assert_eq!(bedrock.region, "us-west-2");
        assert_eq!(bedrock.profile.as_deref(), Some("some-profile"));
        assert_eq!(bedrock.model_for(Tier::Opus), Some("opus-arn"));
        assert_eq!(bedrock.model_for(Tier::Haiku), Some("haiku-arn"));
    }

    /// Absent unless asked for, or every existing build would start signing for a service it has
    /// no credentials for.
    #[test]
    fn nothing_configured_is_not_bedrock() {
        assert_eq!(configured(&[]), None);
    }

    /// Only `1` turns it on. A variable someone set to `0` or `false` to disable it must not read
    /// as enabled, which is what testing for mere presence would do.
    #[test]
    fn only_one_turns_it_on() {
        for value in ["0", "", "false", "no", "true", "yes", " "] {
            assert_eq!(
                configured(&[
                    (env_var::USE_BEDROCK, value),
                    (env_var::AWS_REGION, "us-west-2"),
                ]),
                None,
                "{value:?} was read as enabled"
            );
        }
    }

    /// Without a region there is no host and nothing to sign for. Falling back to the working
    /// backend beats guessing a region and failing far from the mistake.
    #[test]
    fn the_switch_without_a_region_is_not_configured() {
        assert_eq!(configured(&[(env_var::USE_BEDROCK, "1")]), None);
        assert_eq!(
            configured(&[(env_var::USE_BEDROCK, "1"), (env_var::AWS_REGION, "   ")]),
            None
        );
    }

    /// A profile is optional: a machine with instance credentials or a bare access key has none,
    /// and the AWS CLI resolves those the same way it would for any other command.
    #[test]
    fn a_profile_is_optional() {
        let bedrock = configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
        ])
        .expect("configured");
        assert_eq!(bedrock.profile, None);
    }

    /// A tier with no variable set is one this configuration cannot reach. An ARN cannot be derived
    /// from a model name, so a guessed one is a request that fails at the far end.
    #[test]
    fn an_unnamed_tier_is_left_out_rather_than_guessed() {
        let bedrock = configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_SONNET_MODEL, "sonnet-arn"),
        ])
        .expect("configured");
        assert_eq!(bedrock.model_for(Tier::Opus), None);
        assert_eq!(bedrock.model_for(Tier::Haiku), None);
        assert_eq!(bedrock.models().len(), 1);
    }

    /// The strongest configured tier, not the cheapest: someone who named three tiers asked for the
    /// best of them, and quietly answering with the weakest is a downgrade nobody was told about.
    #[test]
    fn the_default_model_is_the_strongest_configured_tier() {
        let all = configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
            (env_var::BEDROCK_SONNET_MODEL, "sonnet-arn"),
        ])
        .expect("configured");
        assert_eq!(all.default_model(), Some("opus-arn"));

        let weaker = configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_HAIKU_MODEL, "haiku-arn"),
        ])
        .expect("configured");
        assert_eq!(weaker.default_model(), Some("haiku-arn"));
    }

    /// A block that turns Bedrock on and names nothing is still Bedrock. Reporting "no models
    /// configured" is better than inventing one, and better than silently using the other backend
    /// when the person asked for this one.
    #[test]
    fn bedrock_with_no_models_is_still_bedrock() {
        let bedrock = configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
        ])
        .expect("configured");
        assert!(bedrock.models().is_empty());
        assert_eq!(bedrock.default_model(), None);
    }

    /// An ARN is full of colons and slashes. Pasted into a path unencoded, its segments are not the
    /// ones intended and the request addresses something else entirely.
    #[test]
    fn a_model_arn_is_encoded_into_the_path() {
        let bedrock = configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
        ])
        .expect("configured");
        let url = bedrock.invoke_url("arn:aws:bedrock:us-west-2:1:foo/bar", false);
        assert!(url.contains("%3A"), "colons survived unencoded: {url}");
        assert!(url.contains("%2F"), "slashes survived unencoded: {url}");
        assert!(
            url.ends_with("/invoke"),
            "the route is no longer the last segment: {url}"
        );
    }

    /// The streamed and unstreamed routes are different paths on the same host, and a client that
    /// asked the wrong one gets a reply in the wrong framing.
    #[test]
    fn streaming_and_buffered_requests_have_different_routes() {
        let bedrock = configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
        ])
        .expect("configured");
        assert!(bedrock.invoke_url("m", false).ends_with("/model/m/invoke"));
        assert!(
            bedrock
                .invoke_url("m", true)
                .ends_with("/model/m/invoke-with-response-stream")
        );
    }

    /// The host is derived from the region, and it is what SigV4 signs. A mismatch between the two
    /// is a signature the service rejects.
    #[test]
    fn the_host_names_the_configured_region() {
        let bedrock = configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "eu-central-1"),
        ])
        .expect("configured");
        assert_eq!(bedrock.host(), "bedrock-runtime.eu-central-1.amazonaws.com");
    }

    /// A remembered choice outlives the settings that made it reachable. Bedrock rejects an unknown
    /// model rather than substituting one, so a stale name has to be caught before it is sent.
    #[test]
    fn a_model_that_is_not_configured_is_not_offered() {
        let bedrock = configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
        ])
        .expect("configured");
        assert!(bedrock.offers("opus-arn"));
        assert!(!bedrock.offers("some-model-that-was-removed"));
    }
}
