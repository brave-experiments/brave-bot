//! Capabilities and the labels their output carries.
//!
//! A capability names a class of observation or effect. Every capability that
//! *produces* data declares the label that data arrives with, so a fetcher cannot
//! decide its own output is trustworthy.
//!
//! The set here is deliberately coding-shaped. Anything domain-specific belongs
//! behind MCP rather than in this enum. See [`Capability::WebFetch`] for the one
//! general-purpose fetch primitive.

use crate::label::Label;
use std::fmt;

/// A named capability. Holding the corresponding [`CapabilityToken`] is what permits
/// an operation; the enum itself is just an identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Capability {
    /// Read a file from the workspace.
    ///
    /// Output is untrusted: a file may contain anything, including text fetched from
    /// the network by an earlier step. It is private because workspace contents are
    /// the user's and must not leave without declassification.
    FileRead,
    /// Write a file in the workspace. An effect, not an observation.
    FileWrite,
    /// Execute a subprocess. The most dangerous capability: its argument is
    /// simultaneously destination and payload, so it cannot be split into routing and
    /// content the way a file write can.
    ShellExec,
    /// Read repository state: log, diff, status.
    GitRead,
    /// Mutate repository state: commit, branch, tag.
    GitWrite,
    /// Fetch a URL. Output is untrusted and public: it is attacker-influenceable but
    /// carries no confidentiality of ours.
    WebFetch,
    /// Call a tool on an MCP server. Output is untrusted; confidentiality depends on
    /// the server, so this label is the conservative floor and a server may raise it.
    McpCall,
}

impl Capability {
    /// The label data produced by this capability arrives with.
    ///
    /// `None` for pure effects, which produce no observation to label.
    pub fn output_label(self) -> Option<Label> {
        match self {
            // Workspace content is ours (private) and may contain anything (untrusted).
            Self::FileRead | Self::GitRead => Some(Label::untrusted_private()),
            // Remote content is attacker-influenceable but not confidential to us.
            Self::WebFetch | Self::McpCall => Some(Label::untrusted_public()),
            // Effects produce no labelled observation.
            Self::FileWrite | Self::GitWrite => None,
            // Command output can contain anything the workspace contains.
            Self::ShellExec => Some(Label::untrusted_private()),
        }
    }

    /// Whether this capability changes the world, as opposed to observing it.
    ///
    /// Effects are what the action gates guard; observations only need labelling.
    pub fn is_effect(self) -> bool {
        matches!(
            self,
            Self::FileWrite | Self::GitWrite | Self::ShellExec | Self::McpCall
        )
    }

    /// Whether this capability requires network egress, and so must pass through the
    /// single egress chokepoint.
    pub fn needs_network(self) -> bool {
        matches!(self, Self::WebFetch | Self::McpCall)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileRead => "file_read",
            Self::FileWrite => "file_write",
            Self::ShellExec => "shell_exec",
            Self::GitRead => "git_read",
            Self::GitWrite => "git_write",
            Self::WebFetch => "web_fetch",
            Self::McpCall => "mcp_call",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Proof that a capability was granted.
///
/// Cannot be constructed outside this crate, so downstream code cannot forge a grant
/// It must receive one from a [`CapabilitySet`] built by the policy layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityToken {
    capability: Capability,
}

impl CapabilityToken {
    pub(crate) fn mint(capability: Capability) -> Self {
        Self { capability }
    }

    pub fn capability(self) -> Capability {
        self.capability
    }
}

/// The capabilities granted for one run.
///
/// Deliberately immutable once built: a run cannot acquire new capabilities partway
/// through, which is what stops a compromised step from escalating.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySet {
    granted: Vec<Capability>,
}

impl CapabilitySet {
    /// An empty set. Grants nothing, which is the right default for untrusted work.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn contains(&self, capability: Capability) -> bool {
        self.granted.contains(&capability)
    }

    pub fn is_empty(&self) -> bool {
        self.granted.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.granted.iter().copied()
    }

    /// Hand out a token if this capability was granted.
    ///
    /// The token is the only way to satisfy an operation that requires a capability,
    /// so a caller cannot proceed by asserting it has permission.
    pub fn token_for(&self, capability: Capability) -> Option<CapabilityToken> {
        self.contains(capability)
            .then(|| CapabilityToken::mint(capability))
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<I: IntoIterator<Item = Capability>>(capabilities: I) -> Self {
        let mut granted: Vec<_> = capabilities.into_iter().collect();
        granted.sort();
        granted.dedup();
        Self { granted }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observations_are_untrusted() {
        for c in [
            Capability::FileRead,
            Capability::GitRead,
            Capability::WebFetch,
            Capability::McpCall,
            Capability::ShellExec,
        ] {
            let label = c.output_label().expect("observation must have a label");
            assert!(!label.is_trusted(), "{c} output must not be trusted");
        }
    }

    /// Nothing a capability produces is ever routing-safe. Routing must come from
    /// trusted input, so if any capability yielded `(T,pub)` the asymmetry would leak.
    #[test]
    fn no_capability_produces_routing_safe_output() {
        for c in [
            Capability::FileRead,
            Capability::FileWrite,
            Capability::ShellExec,
            Capability::GitRead,
            Capability::GitWrite,
            Capability::WebFetch,
            Capability::McpCall,
        ] {
            if let Some(label) = c.output_label() {
                assert_ne!(
                    label,
                    Label::trusted_public(),
                    "{c} must not produce routing-safe output"
                );
            }
        }
    }

    #[test]
    fn workspace_reads_are_private_and_remote_reads_are_public() {
        assert_eq!(
            Capability::FileRead.output_label(),
            Some(Label::untrusted_private())
        );
        assert_eq!(
            Capability::WebFetch.output_label(),
            Some(Label::untrusted_public())
        );
    }

    #[test]
    fn pure_effects_have_no_output_label() {
        assert_eq!(Capability::FileWrite.output_label(), None);
        assert_eq!(Capability::GitWrite.output_label(), None);
    }

    #[test]
    fn effects_and_observations_are_distinguished() {
        assert!(Capability::FileWrite.is_effect());
        assert!(Capability::ShellExec.is_effect());
        assert!(!Capability::FileRead.is_effect());
        assert!(!Capability::WebFetch.is_effect());
    }

    #[test]
    fn network_capabilities_are_identified() {
        assert!(Capability::WebFetch.needs_network());
        assert!(Capability::McpCall.needs_network());
        assert!(!Capability::FileRead.needs_network());
        assert!(!Capability::ShellExec.needs_network());
    }

    #[test]
    fn an_empty_set_grants_nothing() {
        let set = CapabilitySet::none();
        assert!(set.is_empty());
        assert!(set.token_for(Capability::FileRead).is_none());
    }

    #[test]
    fn a_token_is_issued_only_for_granted_capabilities() {
        let set = CapabilitySet::from_iter([Capability::FileRead]);
        let token = set.token_for(Capability::FileRead).expect("granted");
        assert_eq!(token.capability(), Capability::FileRead);
        assert!(set.token_for(Capability::FileWrite).is_none());
    }

    #[test]
    fn duplicate_grants_collapse() {
        let set = CapabilitySet::from_iter([
            Capability::FileRead,
            Capability::FileRead,
            Capability::ShellExec,
        ]);
        assert_eq!(set.iter().count(), 2);
    }
}
