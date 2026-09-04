// Environment variable names, kept together so the set is auditable at a glance.
//
// The build script includes this file directly, so it must stay dependency-free:
// constants only.

pub const SIGNING_KEY: &str = "SERVICES_KEY_AICHAT";
pub const KEY_ID: &str = "BRAVE_SERVICES_KEY_ID";
pub const ENDPOINT: &str = "BRAVE_AI_CHAT_ENDPOINT";
/// Host for the premium tier, used once a subscription has been imported.
///
/// A separate variable rather than a prefix swap on ENDPOINT: the two are independent
/// deployments, and deriving one host from another by string substitution would break
/// the moment either name changed.
pub const PREMIUM_ENDPOINT: &str = "BRAVE_AI_CHAT_PREMIUM_ENDPOINT";
/// The model to request when the user has not chosen one.
///
/// A default rather than the model: `/model` picks one per user and that choice wins, so this
/// applies until someone makes one. Prefixed like the other Brave variables, because an
/// unqualified `MODEL` in a shared shell profile collides with whatever else wanted the name.
pub const DEFAULT_MODEL: &str = "BRAVE_AI_CHAT_DEFAULT_MODEL";

/// How many prompt tokens a conversation may reach before it is compacted.
///
/// Deliberately absent from ALL, so it is never baked into a binary. The others are credentials
/// and hosts, which belong to the build; this is a knob one person turns while working, and a
/// value someone exported to debug a session would otherwise ship to everyone who used their
/// release.
pub const CONTEXT_BUDGET: &str = "BRAVEBOT_CONTEXT_BUDGET";

/// Set to `1` to reach Claude through AWS Bedrock rather than the aichat backend.
///
/// Brave-prefixed like the rest of this file: the switch decides which backend this program uses,
/// so it belongs to this program. The model names below keep Claude Code's spelling, since those
/// name someone's Bedrock deployment rather than anything here.
///
/// Absent from ALL, along with the rest of the Bedrock names. They describe one person's AWS
/// account, not the build.
pub const USE_BEDROCK: &str = "BRAVEBOT_USE_BEDROCK";

/// Which AWS region to reach Bedrock in. Required once [`USE_BEDROCK`] is on.
pub const AWS_REGION: &str = "AWS_REGION";

/// Which profile in the AWS configuration names the credentials to sign with.
///
/// Optional: with nothing set the AWS CLI resolves credentials as it would for any other command,
/// which is what a machine using instance credentials or a bare access key already relies on.
pub const AWS_PROFILE: &str = "AWS_PROFILE";

/// The model each tier names, as either a model id or an inference-profile ARN.
///
/// A tier whose variable is unset is one this configuration cannot reach, and it is left out rather
/// than guessed at: an ARN cannot be derived from a model name, so inventing one would produce a
/// request that fails at the far end for a reason nothing here could explain.
pub const BEDROCK_OPUS_MODEL: &str = "ANTHROPIC_DEFAULT_OPUS_MODEL";
pub const BEDROCK_SONNET_MODEL: &str = "ANTHROPIC_DEFAULT_SONNET_MODEL";
pub const BEDROCK_HAIKU_MODEL: &str = "ANTHROPIC_DEFAULT_HAIKU_MODEL";

/// Every name the Bedrock backend reads, in the order `doctor` reports them.
pub const BEDROCK_ALL: [&str; 6] = [
    USE_BEDROCK,
    AWS_REGION,
    AWS_PROFILE,
    BEDROCK_OPUS_MODEL,
    BEDROCK_SONNET_MODEL,
    BEDROCK_HAIKU_MODEL,
];

/// Every name a build may bake in, in the order `doctor` reports them.
pub const ALL: [&str; 5] = [SIGNING_KEY, KEY_ID, ENDPOINT, PREMIUM_ENDPOINT, DEFAULT_MODEL];

/// Names a build must have.
///
/// DEFAULT_MODEL is absent because it has a default. PREMIUM_ENDPOINT is absent because a build
/// without it still works for everyone who has not imported a subscription.
pub const REQUIRED: [&str; 3] = [SIGNING_KEY, KEY_ID, ENDPOINT];

/// Set to `1` to build without configuration, producing a binary that must be given
/// the variables at run time.
pub const ALLOW_UNCONFIGURED_BUILD: &str = "BRAVEBOT_ALLOW_UNCONFIGURED_BUILD";
