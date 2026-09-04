//! Which backend a request goes to.
//!
//! Three exist: the aichat endpoint Brave runs, Claude on AWS Bedrock, and an OpenAI-compatible
//! gateway somebody configured. The choice is made from configuration, once, here, so the turn loop
//! and everything beside it asks the same question of whichever one is in use rather than branching
//! on the backend at every call.
//!
//! The choice is not content. It comes from [`bravebot_config::Config`], which is built from the
//! environment and the user's own settings file before any turn starts, and nothing a model says can
//! reach it.

use bravebot_aichat::protocol::ChatRequest;
use bravebot_aichat::{AichatClient, ChatError, Completion, Progress, Subscription};
use bravebot_bedrock::{BedrockClient, BedrockError};
use bravebot_config::Config;
use bravebot_core::cancel::Cancel;
use bravebot_core::event::Sink;
use bravebot_core::policy::Policy;
use bravebot_net::Egress;
use std::fmt;

/// A failure from whichever backend was asked.
///
/// One type so a caller handles one error. The variants stay distinct because the remedies differ:
/// an expired AWS session is fixed by signing in, and a spent Leo credential by importing again.
#[derive(Debug)]
pub enum BackendError {
    Aichat(ChatError),
    Bedrock(BedrockError),
    /// A gateway is configured but nothing holds its bearer token.
    ///
    /// Refused here rather than sent unauthenticated, on the same footing as a Bedrock tier whose
    /// model cannot be resolved: a request the configuration cannot sign fails at the far end for a
    /// reason nothing local could explain, and the remedy is naming a variable that holds one.
    NoGatewayToken {
        provider: String,
    },
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aichat(e) => write!(f, "{e}"),
            Self::Bedrock(e) => write!(f, "{e}"),
            Self::NoGatewayToken { provider } => write!(
                f,
                "no credential for the {provider} gateway: set one of the variables its `env` names, \
                 or `options.apiKey` in its settings block"
            ),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<ChatError> for BackendError {
    fn from(value: ChatError) -> Self {
        Self::Aichat(value)
    }
}

impl From<BedrockError> for BackendError {
    fn from(value: BedrockError) -> Self {
        Self::Bedrock(value)
    }
}

impl BackendError {
    /// Whether this is the caller's own stop arriving back rather than a failure.
    ///
    /// Both backends have their own way of saying it, and a stop reported as a failure is written
    /// into the transcript as something that went wrong with the model.
    pub fn is_cancelled(&self) -> bool {
        matches!(
            self,
            Self::Aichat(ChatError::Cancelled) | Self::Bedrock(BedrockError::Cancelled)
        )
    }
}

/// The backend this configuration selects, ready to be asked for one reply.
///
/// Holds no credential. Both backends resolve their own when a request is built, which for Bedrock
/// matters: a session expires mid-run, and a key read once at startup would stop working part way
/// through.
pub enum Backend<'a> {
    Aichat {
        config: &'a Config,
        egress: &'a Egress,
        subscription: Option<&'a mut dyn Subscription>,
        cancel: Option<Cancel>,
    },
    Bedrock {
        config: &'a bravebot_config::bedrock::Bedrock,
        egress: &'a Egress,
        cancel: Option<Cancel>,
    },
    /// A configured OpenAI-compatible gateway.
    ///
    /// Speaks the same protocol as the aichat backend and is sent by the same client, differing in
    /// host and credential. A separate variant regardless, because which service a request went to is
    /// the thing worth being able to read at a dispatch site.
    Gateway {
        config: &'a Config,
        provider: &'a bravebot_config::provider::Provider,
        /// What this gateway calls the model, which is the name it is asked for.
        ///
        /// Carried because a request may name the model as `openrouter/z-ai/glm-4.6`, which says
        /// which service is meant and is a name that service has never heard of. Resolved once,
        /// where the provider is found, so no later step has to know the difference.
        wire_model: String,
        egress: &'a Egress,
        cancel: Option<Cancel>,
    },
}

impl<'a> Backend<'a> {
    /// The backend that serves `model`.
    ///
    /// The model decides, not the configuration. Both rosters are offered at once when a Bedrock
    /// block is present, so a name has to be sent to the backend that recognises it: Bedrock rejects
    /// an unknown model rather than substituting one, and the aichat endpoint has never heard of an
    /// inference-profile ARN.
    ///
    /// Not content. The name comes from what `/model` listed and a person picked, or from the
    /// configured default, and the pick is the endorsement for the request field it lands in.
    pub fn select(config: &'a Config, egress: &'a Egress, model: &str) -> Self {
        if let Some(bedrock) = config.bedrock.as_ref().filter(|it| it.offers(model)) {
            return Self::Bedrock {
                config: bedrock,
                egress,
                cancel: None,
            };
        }
        if let Some((provider, wire_model)) = config.provider_for(model) {
            return Self::Gateway {
                config,
                provider,
                wire_model: wire_model.to_string(),
                egress,
                cancel: None,
            };
        }
        Self::Aichat {
            config,
            egress,
            subscription: None,
            cancel: None,
        }
    }

    /// Sign in for `model`, where its backend needs that and has no usable session.
    ///
    /// Here rather than in an interface because which backend answers, and whether it authenticates
    /// interactively at all, is this module's question. A caller asks once, before it starts work,
    /// and does not learn which service it was for.
    ///
    /// `say` is called per line as the sign-in writes it, while it is still waiting: a URL and a code
    /// to type into it. A caller shows them, because they are the flow rather than a report of it.
    /// Never called where no sign-in is needed, which is every turn but the first of a day, so this
    /// is cheap enough to ask before each one.
    pub fn sign_in_if_needed(
        config: &Config,
        model: &str,
        say: impl FnMut(String),
    ) -> Result<(), BackendError> {
        let Some(bedrock) = config.bedrock.as_ref().filter(|it| it.offers(model)) else {
            return Ok(());
        };
        bravebot_bedrock::credentials::sign_in_if_needed(bedrock.profile.as_deref(), say)
            .map_err(|failure| BedrockError::Credentials(failure).into())
    }

    /// Whether the name a request carries and the name its reply reports are from the same roster.
    ///
    /// True for the aichat endpoint, which lists concrete models and answers with one of them, so a
    /// difference is a substitution worth telling somebody about. False for Bedrock, where a request
    /// may name an inference-profile ARN: that is a handle standing for whatever the profile resolves
    /// to today, and the reply naming the model behind it is the indirection working. Compared anyway,
    /// it would report a substitution on every single turn.
    ///
    /// True for a gateway too. A configured model is a concrete slug the gateway answers under, and
    /// which upstream served it is a different question from which model did: `provider.order` picks
    /// between hosts of the same model, so a name that comes back changed is a substitution there as
    /// much as on the aichat endpoint.
    pub fn reports_the_model_it_was_asked_for(config: &Config, model: &str) -> bool {
        !config
            .bedrock
            .as_ref()
            .is_some_and(|bedrock| bedrock.offers(model))
    }

    /// The name the service was actually asked for, given the name a session holds.
    ///
    /// The two differ for a gateway, where a session may hold `openrouter/z-ai/glm-4.6` so that the
    /// name says which service is meant, while the request carries the part after the id because that
    /// is what the gateway knows the model by. Everywhere else the name is sent as it stands.
    ///
    /// For comparing against the name a reply reports. Without it every gateway turn looks like a
    /// substitution: the qualified name went in, a bare one came back, and nothing about that is the
    /// service answering with a different model.
    pub fn name_as_asked(config: &Config, model: &str) -> String {
        match config.provider_for(model) {
            Some((_, wire)) => wire.to_string(),
            None => model.to_string(),
        }
    }

    /// Whether [`Backend::sign_in_if_needed`] would do anything, without doing it.
    ///
    /// For an interface deciding whether to say something first. Asking separately rather than
    /// having the sign-in report back, because what it would return is "nothing happened", and a
    /// caller that has already dismantled its display to find that out has paid the whole cost.
    pub fn needs_sign_in(config: &Config, model: &str) -> bool {
        config
            .bedrock
            .as_ref()
            .filter(|it| it.offers(model))
            .is_some_and(|bedrock| {
                !bravebot_bedrock::credentials::is_signed_in(bedrock.profile.as_deref())
            })
    }

    /// Send requests on the premium tier, where the backend has one.
    ///
    /// Ignored by Bedrock, which has no such notion: an AWS account reaches the models it reaches,
    /// and a Leo credential means nothing to it.
    pub fn with_subscription(mut self, source: &'a mut dyn Subscription) -> Self {
        if let Self::Aichat { subscription, .. } = &mut self {
            *subscription = Some(source);
        }
        self
    }

    /// Stop reading a streamed reply as soon as this says to.
    pub fn with_cancel(mut self, stop: Cancel) -> Self {
        match &mut self {
            Self::Aichat { cancel, .. }
            | Self::Bedrock { cancel, .. }
            | Self::Gateway { cancel, .. } => *cancel = Some(stop),
        }
        self
    }

    /// Send a request and wait for the whole reply.
    pub fn complete<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
    ) -> Result<Completion, BackendError> {
        match self {
            Self::Aichat {
                config,
                egress,
                subscription,
                cancel,
            } => {
                let mut client = AichatClient::new(config, egress);
                if let Some(cancel) = cancel {
                    client = client.with_cancel(cancel.clone());
                }
                if let Some(subscription) = subscription.as_mut() {
                    client = client.with_subscription(*subscription);
                }
                Ok(client.complete(policy, request)?)
            }
            Self::Bedrock {
                config,
                egress,
                cancel,
            } => {
                let mut client = BedrockClient::new(config, egress);
                if let Some(cancel) = cancel {
                    client = client.with_cancel(cancel.clone());
                }
                Ok(client.complete(policy, request)?)
            }
            Self::Gateway {
                config,
                provider,
                wire_model,
                egress,
                cancel,
            } => {
                let mut client = gateway_client(config, provider, wire_model, egress)?;
                if let Some(cancel) = cancel {
                    client = client.with_cancel(cancel.clone());
                }
                Ok(client.complete(policy, request)?)
            }
        }
    }

    /// Send a request and read the reply as it arrives.
    pub fn complete_streaming<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
        progress: impl FnMut(Progress),
    ) -> Result<Completion, BackendError> {
        match self {
            Self::Aichat {
                config,
                egress,
                subscription,
                cancel,
            } => {
                let mut client = AichatClient::new(config, egress);
                if let Some(cancel) = cancel {
                    client = client.with_cancel(cancel.clone());
                }
                if let Some(subscription) = subscription.as_mut() {
                    client = client.with_subscription(*subscription);
                }
                Ok(client.complete_streaming(policy, request, progress)?)
            }
            Self::Bedrock {
                config,
                egress,
                cancel,
            } => {
                let mut client = BedrockClient::new(config, egress);
                if let Some(cancel) = cancel {
                    client = client.with_cancel(cancel.clone());
                }
                Ok(client.complete_streaming(policy, request, progress)?)
            }
            Self::Gateway {
                config,
                provider,
                wire_model,
                egress,
                cancel,
            } => {
                let mut client = gateway_client(config, provider, wire_model, egress)?;
                if let Some(cancel) = cancel {
                    client = client.with_cancel(cancel.clone());
                }
                Ok(client.complete_streaming(policy, request, progress)?)
            }
        }
    }
}

/// A client pointed at `provider`, with its token resolved.
///
/// The token is read here rather than held on the variant, because it is read from the process
/// environment at the point a request needs signing: a value read once at startup would go stale in a
/// session where somebody exported a new one.
fn gateway_client<'a>(
    config: &'a Config,
    provider: &'a bravebot_config::provider::Provider,
    wire_model: &str,
    egress: &'a Egress,
) -> Result<AichatClient<'a>, BackendError> {
    let token = provider
        .token(|name| std::env::var(name).ok())
        .ok_or_else(|| BackendError::NoGatewayToken {
            provider: provider.display_name().to_string(),
        })?;
    Ok(AichatClient::new(config, egress).for_gateway(provider, wire_model, token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_config::env_var;

    fn aichat_only(key: &str) -> Option<String> {
        match key {
            env_var::SIGNING_KEY => Some("test-signing-key"),
            env_var::KEY_ID => Some("test-key-id"),
            env_var::ENDPOINT => Some("https://example.invalid"),
            _ => None,
        }
        .map(str::to_string)
    }

    /// Both rosters at once, so a configuration alone cannot say where a request goes.
    fn both_backends() -> Config {
        Config::from_lookup(|key| match key {
            env_var::USE_BEDROCK => Some("1".to_string()),
            env_var::AWS_REGION => Some("us-west-2".to_string()),
            env_var::BEDROCK_OPUS_MODEL => Some("opus-arn".to_string()),
            // A profile no machine has, so whether an AWS session exists is a property of this
            // configuration rather than of whoever is running the tests. Left unset, the CLI falls
            // back to ambient credentials and a developer with a live session would see a different
            // answer from one without.
            env_var::AWS_PROFILE => Some("a-profile-no-machine-has".to_string()),
            other => aichat_only(other),
        })
        .expect("configured")
    }

    /// Every roster at once, so a configuration alone cannot say where a request goes.
    fn with_a_gateway() -> Config {
        let mut config = both_backends();
        let serde_json::Value::Object(root) = serde_json::from_str(
            r#"{"provider": {"openrouter": {
                "env": ["A_TOKEN_VARIABLE"],
                "options": {"baseURL": "https://openrouter.example.invalid/api/v1"},
                "models": {"z-ai/glm-4.6": {}, "anthropic/claude-sonnet-4.5": {}}
            }}}"#,
        )
        .expect("json") else {
            panic!("not an object");
        };
        config.providers = bravebot_config::provider::Provider::all(&root);
        config
    }

    /// The model names the service. A gateway slug means nothing to Brave's endpoint or to Bedrock,
    /// so a request carrying one has to reach the gateway that listed it.
    #[test]
    fn a_configured_gateway_model_selects_the_gateway_backend() {
        let egress = Egress::new();
        assert!(matches!(
            Backend::select(&with_a_gateway(), &egress, "z-ai/glm-4.6"),
            Backend::Gateway { .. }
        ));
    }

    /// Configuring a gateway is additive. A name it does not list still reaches the service that
    /// does, which is the whole of what "takes nothing away" means here.
    #[test]
    fn a_gateway_does_not_take_the_other_rosters_away() {
        let config = with_a_gateway();
        let egress = Egress::new();
        assert!(matches!(
            Backend::select(&config, &egress, "automatic"),
            Backend::Aichat { .. }
        ));
        assert!(matches!(
            Backend::select(&config, &egress, "opus-arn"),
            Backend::Bedrock { .. }
        ));
    }

    /// A gateway authenticates with a bearer token and has nothing to sign in for. Deciding
    /// otherwise would run the AWS CLI before a request that never touches AWS.
    #[test]
    fn a_gateway_model_never_needs_an_aws_sign_in() {
        let config = with_a_gateway();
        assert!(!Backend::needs_sign_in(&config, "z-ai/glm-4.6"));
        let mut said = Vec::new();
        assert!(
            Backend::sign_in_if_needed(&config, "z-ai/glm-4.6", |line| said.push(line)).is_ok()
        );
        assert!(said.is_empty(), "a gateway asked for a sign-in: {said:?}");
    }

    /// A gateway answers under the slug it was asked for, so a name that comes back changed is a
    /// substitution worth reporting. Unlike a Bedrock ARN, there is no indirection to explain it.
    #[test]
    fn a_gateway_reports_the_model_it_was_asked_for() {
        assert!(Backend::reports_the_model_it_was_asked_for(
            &with_a_gateway(),
            "z-ai/glm-4.6"
        ));
    }

    /// A request the configuration cannot sign is refused rather than sent unauthenticated, which
    /// would fail at the far end for a reason nothing local could explain.
    #[test]
    fn a_gateway_with_nothing_holding_a_token_refuses_the_request() {
        let config = with_a_gateway();
        let egress = Egress::new();
        let (provider, wire) = config.provider_for("z-ai/glm-4.6").expect("offered");
        let failure = gateway_client(&config, provider, wire, &egress)
            .err()
            .expect("refused");
        assert!(matches!(failure, BackendError::NoGatewayToken { .. }));
        assert!(
            format!("{failure}").contains("openrouter"),
            "the failure does not say which gateway: {failure}"
        );
    }

    /// Nothing changes for a build that was not pointed at Bedrock, which is every existing one.
    #[test]
    fn without_bedrock_configured_the_aichat_backend_is_selected() {
        let config = Config::from_lookup(aichat_only).expect("configured");
        let egress = Egress::new();
        assert!(matches!(
            Backend::select(&config, &egress, "automatic"),
            Backend::Aichat { .. }
        ));
    }

    /// A configured tier goes to Bedrock. The aichat endpoint has never heard of an
    /// inference-profile ARN, so sending one there is a request that cannot be served.
    #[test]
    fn a_configured_bedrock_model_selects_the_bedrock_backend() {
        let egress = Egress::new();
        assert!(matches!(
            Backend::select(&both_backends(), &egress, "opus-arn"),
            Backend::Bedrock { .. }
        ));
    }

    /// A model Brave serves needs no AWS session, whatever else is configured. Deciding otherwise
    /// would run the AWS CLI, and on the interactive path would hand a person's screen to a sign-in
    /// for a service the turn was never going to touch.
    #[test]
    fn a_brave_model_never_needs_an_aws_sign_in() {
        for model in ["automatic", "claude-3-sonnet"] {
            assert!(
                !Backend::needs_sign_in(&both_backends(), model),
                "{model} asked for a sign-in"
            );
        }
    }

    /// A build with no AWS configuration has nothing to sign in to, so the question is answered
    /// without asking anything of the machine.
    #[test]
    fn without_bedrock_configured_nothing_needs_a_sign_in() {
        let config = Config::from_lookup(aichat_only).expect("configured");
        assert!(!Backend::needs_sign_in(&config, "automatic"));
        assert!(!Backend::needs_sign_in(&config, "opus-arn"));
    }

    /// Signing in is a no-op for anything this configuration does not serve, so a caller may ask
    /// before every turn without a thought for which backend is about to answer. Nothing is said
    /// either: the lines exist to walk somebody through a sign-in, and there is no sign-in.
    #[test]
    fn signing_in_for_a_model_no_aws_account_serves_does_nothing() {
        let config = Config::from_lookup(aichat_only).expect("configured");
        let mut said = Vec::new();
        assert!(Backend::sign_in_if_needed(&config, "automatic", |line| said.push(line)).is_ok());
        assert!(
            Backend::sign_in_if_needed(&both_backends(), "claude-3-sonnet", |line| said.push(line))
                .is_ok()
        );
        assert!(said.is_empty(), "{said:?}");
    }

    /// The aichat endpoint lists concrete models and answers with one of them, so a name that comes
    /// back different is a substitution somebody should be told about.
    #[test]
    fn a_brave_model_is_expected_to_be_the_one_reported() {
        let config = Config::from_lookup(aichat_only).expect("configured");
        assert!(Backend::reports_the_model_it_was_asked_for(
            &config,
            "claude-3-sonnet"
        ));
    }

    /// An inference-profile ARN is a handle standing for whatever it resolves to, so the reply naming
    /// a different model is that working rather than a substitution. Compared anyway, every turn on
    /// Bedrock reported one.
    #[test]
    fn a_bedrock_arn_is_not_expected_to_be_the_name_that_comes_back() {
        assert!(!Backend::reports_the_model_it_was_asked_for(
            &both_backends(),
            "opus-arn"
        ));
    }

    /// The model decides, not the block. A Bedrock block adds a roster rather than diverting the
    /// whole of one: with both offered, picking a Brave model must still reach Brave.
    #[test]
    fn a_brave_model_still_reaches_aichat_while_bedrock_is_configured() {
        let egress = Egress::new();
        for model in ["automatic", "claude-3-sonnet"] {
            assert!(
                matches!(
                    Backend::select(&both_backends(), &egress, model),
                    Backend::Aichat { .. }
                ),
                "{model} was diverted to Bedrock"
            );
        }
    }

    /// A stop is the person's own, arriving back. Reported as a failure it is written into the
    /// transcript as something that went wrong with the model, so both backends' ways of saying it
    /// have to be recognised.
    #[test]
    fn a_stop_from_either_backend_is_recognised_as_a_stop() {
        assert!(BackendError::from(ChatError::Cancelled).is_cancelled());
        assert!(BackendError::from(BedrockError::Cancelled).is_cancelled());
    }

    /// Anything else is a real failure and must not be mistaken for a stop, or a turn that broke
    /// would look like one the person interrupted.
    #[test]
    fn other_failures_are_not_mistaken_for_a_stop() {
        assert!(!BackendError::from(ChatError::NoContent).is_cancelled());
        assert!(!BackendError::from(BedrockError::NoContent).is_cancelled());
        assert!(!BackendError::from(BedrockError::TooLong).is_cancelled());
    }

    /// The remedies differ, so the messages have to. An expired AWS session is fixed by signing in
    /// and a spent Leo credential by importing again, and a person shown the wrong one is sent to
    /// fix something that is not broken.
    #[test]
    fn each_backend_keeps_its_own_explanation() {
        let bedrock = BackendError::from(BedrockError::Credentials(
            bravebot_bedrock::credentials::CredentialError::Refused {
                detail: "the session has expired".into(),
            },
        ))
        .to_string();
        assert!(bedrock.contains("aws sso login"), "{bedrock}");

        let leo = BackendError::from(ChatError::Subscription("spent".into())).to_string();
        assert!(leo.contains("import-leo-creds"), "{leo}");
    }
}
