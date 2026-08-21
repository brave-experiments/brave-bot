//! macOS confinement via Seatbelt.
//!
//! Wraps the target command in `sandbox-exec` with a generated deny-by-default
//! profile. The mechanism is what Chrome and Firefox use for their renderer
//! processes; the `sandbox_init` C API is marked deprecated but has no supported
//! replacement for binaries distributed outside the App Store, and `sandbox-exec`
//! remains present on supported macOS versions.
//!
//! Verified empirically: with `(deny default)` a process cannot reach the network
//! (curl fails to resolve or connect) and cannot create files, while still being able
//! to exec and read permitted paths.

use crate::policy::{Capabilities, ConfinementLevel, SandboxPolicy};
use crate::{Sandbox, SandboxError};
use std::path::Path;
use std::process::Command;

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Seatbelt-based confinement.
#[derive(Debug, Default)]
pub struct SeatbeltSandbox;

impl SeatbeltSandbox {
    /// Fails if `sandbox-exec` is missing rather than degrading to no confinement.
    pub fn new() -> Result<Self, SandboxError> {
        if !Path::new(SANDBOX_EXEC).exists() {
            return Err(SandboxError::Unavailable {
                platform: "macos",
                detail: format!("{SANDBOX_EXEC} is not present"),
            });
        }
        Ok(Self)
    }

    /// Build the Seatbelt profile for a policy.
    ///
    /// Starts from `(deny default)` and adds only what the policy grants. Paths are
    /// written as subpath rules so a granted directory covers its contents.
    pub fn profile(policy: &SandboxPolicy) -> String {
        let mut out = String::from("(version 1)\n(deny default)\n");

        // Without these the process cannot start at all: the loader must exec the
        // binary and mach lookups are needed for basic runtime services. They grant no
        // filesystem or network reach of their own.
        out.push_str("(allow process-exec)\n");
        out.push_str("(allow sysctl-read)\n");
        out.push_str("(allow mach-lookup)\n");

        // The dynamic loader reads the root directory entry itself, which a subpath
        // grant for e.g. /usr does not cover. Without this the process dies with
        // SIGABRT before main runs, which looks like a mysterious crash rather than a
        // denied read. Reading `/` alone exposes no file contents.
        out.push_str("(allow file-read* (literal \"/\"))\n");

        for path in &policy.readable {
            out.push_str(&format!(
                "(allow file-read* (subpath {}))\n",
                quote(&path.to_string_lossy())
            ));
        }

        for path in &policy.writable {
            out.push_str(&format!(
                "(allow file-write* (subpath {}))\n",
                quote(&path.to_string_lossy())
            ));
        }

        if policy.allow_network {
            out.push_str("(allow network-outbound)\n");
        }

        if policy.allow_subprocesses {
            out.push_str("(allow process-fork)\n");
        }

        out
    }
}

/// Quote a path as a Seatbelt string literal.
///
/// Escapes backslashes and quotes so a path containing either cannot terminate the
/// literal early and inject profile syntax.
fn quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

impl Sandbox for SeatbeltSandbox {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            // Seatbelt is enforced by the kernel, though its policy language is
            // coarser than Landlock plus seccomp on Linux.
            level: ConfinementLevel::Kernel,
            mechanisms: vec!["seatbelt"],
            network_denial_enforced: true,
        }
    }

    fn command(
        &self,
        program: &str,
        args: &[String],
        policy: &SandboxPolicy,
    ) -> Result<Command, SandboxError> {
        if !policy.is_meaningful() {
            return Err(SandboxError::PolicyTooPermissive);
        }

        let mut wrapped = Command::new(SANDBOX_EXEC);
        wrapped.arg("-p").arg(Self::profile(policy));
        wrapped.arg(program);
        wrapped.args(args);

        // The caller's environment is deliberately not inherited: credentials must never
        // reach a confined process. A caller that needs a variable sets it on the
        // returned command explicitly.
        wrapped.env_clear();

        Ok(wrapped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[test]
    fn a_strict_profile_denies_by_default() {
        let profile = SeatbeltSandbox::profile(&SandboxPolicy::strict());
        assert!(profile.contains("(deny default)"));
        assert!(!profile.contains("network-outbound"));
        assert!(!profile.contains("file-write*"));
        assert!(!profile.contains("process-fork"));
    }

    #[test]
    fn granted_paths_appear_as_subpath_rules() {
        let policy = SandboxPolicy::strict()
            .allow_read("/workspace")
            .allow_write("/workspace/target");
        let profile = SeatbeltSandbox::profile(&policy);
        assert!(profile.contains(r#"(allow file-read* (subpath "/workspace"))"#));
        assert!(profile.contains(r#"(allow file-write* (subpath "/workspace/target"))"#));
    }

    #[test]
    fn network_is_only_allowed_when_requested() {
        let denied = SeatbeltSandbox::profile(&SandboxPolicy::strict());
        assert!(!denied.contains("network-outbound"));

        let allowed = SeatbeltSandbox::profile(&SandboxPolicy::strict().allow_network_egress());
        assert!(allowed.contains("(allow network-outbound)"));
    }

    /// A path containing a quote must not close the string literal and let its
    /// remainder be parsed as profile directives. The payload text still appears — it
    /// is part of the path — but every quote in it is escaped, so Seatbelt reads the
    /// whole thing as one string.
    #[test]
    fn paths_cannot_inject_profile_syntax() {
        let policy = SandboxPolicy::strict().allow_read(r#"/tmp/x") (allow network-outbound) ("#);
        let profile = SeatbeltSandbox::profile(&policy);

        assert!(
            profile.contains(r#"\""#),
            "the embedded quote was not escaped: {profile}"
        );
        // Unescaped, this would have been a directive of its own.
        assert!(
            !profile.contains("\n(allow network-outbound)"),
            "injected directive escaped the literal: {profile}"
        );
    }

    /// Seatbelt must accept a profile built from a hostile path rather than failing to
    /// parse, since a parse failure would be reported as confinement being unavailable.
    #[test]
    fn a_profile_containing_a_hostile_path_still_applies() {
        let sandbox = SeatbeltSandbox::new().expect("sandbox-exec is present on macOS");
        let policy = SandboxPolicy::strict()
            .allow_read("/usr")
            .allow_read("/bin")
            .allow_read(r#"/tmp/x") (allow network-outbound) ("#);

        let mut child = sandbox
            .command("/usr/bin/true", &[], &policy)
            .expect("command builds")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("should spawn");
        assert!(child.wait().expect("should wait").success());
    }

    #[test]
    fn a_fully_permissive_policy_is_refused() {
        let sandbox = SeatbeltSandbox;
        let policy = SandboxPolicy::strict()
            .allow_network_egress()
            .allow_subprocesses()
            .allow_write("/");
        let err = sandbox
            .spawn("/usr/bin/true", &[], &policy)
            .expect_err("must refuse a policy that confines nothing");
        assert!(matches!(err, SandboxError::PolicyTooPermissive));
    }

    #[test]
    fn capabilities_report_kernel_enforcement() {
        let caps = SeatbeltSandbox.capabilities();
        assert_eq!(caps.level, ConfinementLevel::Kernel);
        assert!(caps.network_denial_enforced);
        assert!(caps.mechanisms.contains(&"seatbelt"));
    }

    /// Confirms the sandbox actually runs a process, not just that a profile string
    /// was built.
    #[test]
    fn a_confined_process_runs() {
        let sandbox = SeatbeltSandbox::new().expect("sandbox-exec is present on macOS");
        let policy = SandboxPolicy::strict()
            .allow_read("/usr")
            .allow_read("/bin");
        let mut child = sandbox
            .command("/usr/bin/true", &[], &policy)
            .expect("command builds")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("should spawn");
        assert!(child.wait().expect("should wait").success());
    }

    /// The point of the sandbox: a confined process cannot write outside its grants.
    #[test]
    fn a_confined_process_cannot_write_outside_its_grants() {
        let sandbox = SeatbeltSandbox::new().expect("sandbox-exec is present on macOS");
        let policy = SandboxPolicy::strict()
            .allow_read("/usr")
            .allow_read("/bin");

        let target = std::env::temp_dir().join("bua-sandbox-must-not-exist");
        let _ = std::fs::remove_file(&target);

        let mut child = sandbox
            .command("/usr/bin/touch", &[target.display().to_string()], &policy)
            .expect("command builds")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("should spawn");
        let status = child.wait().expect("should wait");

        assert!(!status.success(), "write should have been denied");
        assert!(!target.exists(), "file was created despite confinement");
    }

    /// Network denial is the property that makes exfiltration structurally impossible,
    /// so it is asserted against a real process rather than only in the profile text.
    #[test]
    fn a_confined_process_cannot_reach_the_network() {
        let sandbox = SeatbeltSandbox::new().expect("sandbox-exec is present on macOS");
        let policy = SandboxPolicy::strict()
            .allow_read("/usr")
            .allow_read("/bin")
            .allow_read("/etc")
            .allow_read("/System")
            .allow_read("/Library");

        let args: Vec<String> = ["-s", "-m", "5", "-o", "/dev/null", "https://example.com"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut child = sandbox
            .command("/usr/bin/curl", &args, &policy)
            .expect("command builds")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("should spawn");
        assert!(
            !child.wait().expect("should wait").success(),
            "network access should have been denied"
        );
    }
}
