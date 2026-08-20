//! What a confined process is allowed to do, and how strongly that is enforced.

use std::fmt;
use std::path::{Path, PathBuf};

/// The confinement a process should run under.
///
/// Deny-by-default: [`SandboxPolicy::strict`] permits nothing, and each allowance is
/// added explicitly. The alternative — starting permissive and subtracting — means a
/// forgotten subtraction silently grants access.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    /// Directories the process may read. Empty means no filesystem reads.
    pub readable: Vec<PathBuf>,
    /// Directories the process may write. Empty means no filesystem writes.
    pub writable: Vec<PathBuf>,
    /// Whether the process may open sockets.
    ///
    /// Normally false. Inference is brokered through the parent, so a confined
    /// process needs no network of its own — and without a socket, an instruction to
    /// exfiltrate data has nowhere to send it.
    pub allow_network: bool,
    /// Whether the process may spawn children. False stops a confined process from
    /// launching an unconfined helper.
    pub allow_subprocesses: bool,
}

impl SandboxPolicy {
    /// Permits nothing: no filesystem, no network, no children.
    pub fn strict() -> Self {
        Self {
            readable: Vec::new(),
            writable: Vec::new(),
            allow_network: false,
            allow_subprocesses: false,
        }
    }

    pub fn allow_read(mut self, path: impl Into<PathBuf>) -> Self {
        self.readable.push(path.into());
        self
    }

    pub fn allow_write(mut self, path: impl Into<PathBuf>) -> Self {
        self.writable.push(path.into());
        self
    }

    /// Permit network access. Named to be conspicuous in review, because granting it
    /// removes the property that makes exfiltration structurally impossible.
    pub fn allow_network_egress(mut self) -> Self {
        self.allow_network = true;
        self
    }

    pub fn allow_subprocesses(mut self) -> Self {
        self.allow_subprocesses = true;
        self
    }

    /// Whether this policy would confine anything at all.
    ///
    /// A policy granting network, subprocesses, and write access to `/` is not
    /// confinement; treating it as such would be the sort of accident that makes a
    /// sandbox decorative.
    pub fn is_meaningful(&self) -> bool {
        !self.allow_network
            || !self.allow_subprocesses
            || !self.writable.iter().any(|p| p.as_path() == Path::new("/"))
    }
}

/// How much confinement was actually achieved.
///
/// Reported rather than assumed: the guarantee genuinely differs across platforms, and
/// claiming a single uniform level would misrepresent the weakest one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfinementLevel {
    /// Kernel-enforced filesystem and network restrictions.
    Kernel,
    /// Restrictions enforced, but coarser than [`ConfinementLevel::Kernel`].
    Partial,
    /// No OS-level confinement available.
    None,
}

impl fmt::Display for ConfinementLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kernel => f.write_str("kernel-enforced"),
            Self::Partial => f.write_str("partial"),
            Self::None => f.write_str("none"),
        }
    }
}

/// What a platform backend can actually deliver, so a caller can tell a user what they
/// got instead of implying every platform is equal.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub level: ConfinementLevel,
    /// Mechanisms in use, for the audit trail.
    pub mechanisms: Vec<&'static str>,
    /// Whether network denial is enforced by the kernel rather than by convention.
    pub network_denial_enforced: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_permits_nothing() {
        let policy = SandboxPolicy::strict();
        assert!(policy.readable.is_empty());
        assert!(policy.writable.is_empty());
        assert!(!policy.allow_network);
        assert!(!policy.allow_subprocesses);
    }

    #[test]
    fn allowances_accumulate() {
        let policy = SandboxPolicy::strict()
            .allow_read("/workspace")
            .allow_write("/workspace/out");
        assert_eq!(policy.readable, vec![PathBuf::from("/workspace")]);
        assert_eq!(policy.writable, vec![PathBuf::from("/workspace/out")]);
    }

    #[test]
    fn a_strict_policy_is_meaningful() {
        assert!(SandboxPolicy::strict().is_meaningful());
    }

    /// Granting everything is not confinement, and must not be mistaken for it.
    #[test]
    fn granting_everything_is_not_meaningful() {
        let policy = SandboxPolicy::strict()
            .allow_network_egress()
            .allow_subprocesses()
            .allow_write("/");
        assert!(!policy.is_meaningful());
    }

    /// Network alone is still confinement if the filesystem stays restricted.
    #[test]
    fn network_alone_remains_meaningful() {
        let policy = SandboxPolicy::strict()
            .allow_network_egress()
            .allow_read("/workspace");
        assert!(policy.is_meaningful());
    }

    #[test]
    fn confinement_levels_render_for_the_audit_trail() {
        assert_eq!(ConfinementLevel::Kernel.to_string(), "kernel-enforced");
        assert_eq!(ConfinementLevel::None.to_string(), "none");
    }
}
