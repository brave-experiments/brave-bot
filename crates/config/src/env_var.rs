// Environment variable names, kept together so the set is auditable at a glance.
//
// The build script includes this file directly, so it must stay dependency-free:
// constants only.

pub const SIGNING_KEY: &str = "SERVICES_KEY_AICHAT";
pub const KEY_ID: &str = "BRAVE_SERVICES_KEY_ID";
pub const ENDPOINT: &str = "BRAVE_AI_CHAT_ENDPOINT";
pub const MODEL: &str = "MODEL";

/// Every name a build may bake in, in the order `doctor` reports them.
pub const ALL: [&str; 4] = [SIGNING_KEY, KEY_ID, ENDPOINT, MODEL];
