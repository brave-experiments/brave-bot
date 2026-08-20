//! OS-level confinement for untrusted subprocesses.
//!
//! One boundary, several backends. The confined process is whatever acts on model
//! output — a processor sub-agent, or a stdio MCP server we launch. Trusted code that
//! performs already-authorised effects is guarded by information-flow gates instead;
//! sandboxing it would confine our own code while leaving the untrusted part free.
//!
//! # Fail closed
//!
//! If confinement cannot be established, [`Sandbox::spawn`] refuses rather than
//! running the process unconfined. Silently degrading is worse than an error: the
//! caller believes it has a guarantee it does not have, and the audit trail records a
//! sandbox that was never applied.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod policy;

use policy::{Capabilities, ConfinementLevel, SandboxPolicy};
use std::fmt;
use std::process::{Child, Command};

#[derive(Debug)]
pub enum SandboxError {
    /// No confinement mechanism is available on this platform or kernel.
    ///
    /// A refusal, not a warning: the process does not run.
    Unavailable {
        platform: &'static str,
        detail: String,
    },
    /// A mechanism exists but could not be applied.
    SetupFailed {
        mechanism: &'static str,
        detail: String,
    },
    /// The policy would not confine anything.
    PolicyTooPermissive,
    /// The process could not be started.
    SpawnFailed(std::io::Error),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { platform, detail } => write!(
                f,
                "no confinement available on {platform} ({detail}); refusing to run \
                 untrusted code unconfined"
            ),
            Self::SetupFailed { mechanism, detail } => write!(
                f,
                "{mechanism} could not be applied ({detail}); refusing to run untrusted \
                 code unconfined"
            ),
            Self::PolicyTooPermissive => f.write_str(
                "the requested policy would not confine anything; refusing to present it \
                 as a sandbox",
            ),
            Self::SpawnFailed(e) => write!(f, "failed to spawn the confined process: {e}"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// A platform confinement backend.
pub trait Sandbox {
    /// What this backend can enforce here, on this kernel.
    fn capabilities(&self) -> Capabilities;

    /// Start a confined process, or refuse.
    fn spawn(&self, command: Command, policy: &SandboxPolicy) -> Result<Child, SandboxError>;
}

/// The backend for the current platform.
///
/// Returns [`SandboxError::Unavailable`] where no backend is implemented, so an
/// unsupported platform is a refusal rather than an unconfined process.
pub fn for_current_platform() -> Result<Box<dyn Sandbox>, SandboxError> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::SeatbeltSandbox::new()?))
    }

    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::LandlockSandbox::new()?))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(SandboxError::Unavailable {
            platform: std::env::consts::OS,
            detail: "no confinement backend is implemented for this platform yet".into(),
        })
    }
}

/// A backend that always refuses.
///
/// Not a fallback: it exists so tests can assert that callers propagate a refusal
/// rather than continuing without confinement.
#[derive(Debug, Default)]
pub struct Unavailable;

impl Sandbox for Unavailable {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            level: ConfinementLevel::None,
            mechanisms: Vec::new(),
            network_denial_enforced: false,
        }
    }

    fn spawn(&self, _command: Command, _policy: &SandboxPolicy) -> Result<Child, SandboxError> {
        Err(SandboxError::Unavailable {
            platform: std::env::consts::OS,
            detail: "confinement is unavailable".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unavailable_backend_refuses_to_spawn() {
        let sandbox = Unavailable;
        let result = sandbox.spawn(Command::new("echo"), &SandboxPolicy::strict());
        assert!(matches!(result, Err(SandboxError::Unavailable { .. })));
    }

    /// The property the whole module exists for: no confinement means no process, not
    /// an unconfined one.
    #[test]
    fn refusal_is_not_a_silent_fallback() {
        let sandbox = Unavailable;
        let err = sandbox
            .spawn(Command::new("echo"), &SandboxPolicy::strict())
            .expect_err("must refuse");
        assert!(err.to_string().contains("refusing to run"));
    }

    #[test]
    fn an_unavailable_backend_reports_no_confinement() {
        let caps = Unavailable.capabilities();
        assert_eq!(caps.level, ConfinementLevel::None);
        assert!(!caps.network_denial_enforced);
    }

    /// Either a real backend is returned, or the lookup refuses. It must never hand
    /// back something that reports no confinement.
    #[test]
    fn the_platform_lookup_never_returns_an_unconfined_backend() {
        match for_current_platform() {
            Ok(sandbox) => assert_ne!(
                sandbox.capabilities().level,
                ConfinementLevel::None,
                "a backend was returned that confines nothing"
            ),
            Err(SandboxError::Unavailable { .. }) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn errors_explain_the_refusal() {
        let err = SandboxError::PolicyTooPermissive;
        assert!(err.to_string().contains("would not confine anything"));
    }
}
