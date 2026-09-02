//! Which backend a request goes to.
//!
//! Two exist: the aichat endpoint Brave runs, and Claude on AWS Bedrock. The choice is made from
//! configuration, once, here, so the turn loop and everything beside it asks the same question of
//! whichever one is in use rather than branching on the backend at every call.
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
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aichat(e) => write!(f, "{e}"),
            Self::Bedrock(e) => write!(f, "{e}"),
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
        announce_login: Option<&'a dyn Fn()>,
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
        match config.bedrock.as_ref() {
            Some(bedrock) if bedrock.offers(model) => Self::Bedrock {
                config: bedrock,
                egress,
                cancel: None,
                announce_login: None,
            },
            _ => Self::Aichat {
                config,
                egress,
                subscription: None,
                cancel: None,
            },
        }
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
            Self::Aichat { cancel, .. } | Self::Bedrock { cancel, .. } => *cancel = Some(stop),
        }
        self
    }

    /// Say something before a browser opens for an AWS sign-in.
    ///
    /// Only Bedrock ever signs in. Without this the first request of the day opens a window with
    /// nothing said about it, which reads as something having gone wrong.
    pub fn announcing_login(mut self, announce: &'a dyn Fn()) -> Self {
        if let Self::Bedrock { announce_login, .. } = &mut self {
            *announce_login = Some(announce);
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
                announce_login,
            } => {
                let mut client = BedrockClient::new(config, egress);
                if let Some(cancel) = cancel {
                    client = client.with_cancel(cancel.clone());
                }
                if let Some(announce) = announce_login {
                    client = client.announcing_login(*announce);
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
                announce_login,
            } => {
                let mut client = BedrockClient::new(config, egress);
                if let Some(cancel) = cancel {
                    client = client.with_cancel(cancel.clone());
                }
                if let Some(announce) = announce_login {
                    client = client.announcing_login(*announce);
                }
                Ok(client.complete_streaming(policy, request, progress)?)
            }
        }
    }
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
            other => aichat_only(other),
        })
        .expect("configured")
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
