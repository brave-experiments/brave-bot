//! Linux confinement via Landlock.
//!
//! Filesystem restrictions are applied with Landlock, which needs kernel 5.13 or
//! newer. Availability is probed at runtime rather than assumed: an older kernel, or a
//! container that masks the syscall, means confinement is unavailable and the process
//! is refused instead of run unconfined.
//!
//! Landlock governs the filesystem only. Network denial needs a separate mechanism
//! (an empty network namespace, or seccomp filtering of `socket`), which is not
//! implemented yet, so [`Capabilities::network_denial_enforced`] is `false` and the
//! reported level is [`ConfinementLevel::Partial`]. Claiming kernel-level network
//! denial here would misreport the guarantee.

use crate::policy::{Capabilities, ConfinementLevel, SandboxPolicy};
use crate::{Sandbox, SandboxError};
use landlock::{
    ABI, Access, AccessFs, CompatLevel, Compatible, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
    path_beneath_rules,
};
use std::os::unix::process::CommandExt;
use std::process::Command;

/// The Landlock ABI this backend targets. ABI v1 is the widest supported set, so
/// confinement works on any kernel from 5.13 onwards rather than only the newest.
const TARGET_ABI: ABI = ABI::V1;

/// `landlock_create_ruleset`, stable since Linux 5.13.
const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;

/// Ask the kernel which Landlock ABI it supports.
///
/// Passing a null attribute pointer with `LANDLOCK_CREATE_RULESET_VERSION` returns the
/// version without creating a ruleset. A negative result means Landlock is absent.
fn landlock_abi_version() -> libc::c_long {
    const LANDLOCK_CREATE_RULESET_VERSION: libc::c_ulong = 1;
    unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            std::ptr::null::<libc::c_void>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    }
}

/// Landlock-based confinement.
#[derive(Debug, Default)]
pub struct LandlockSandbox;

impl LandlockSandbox {
    /// Probes whether Landlock is actually enforceable here.
    ///
    /// Building a ruleset is not a sufficient test: under `BestEffort` the crate will
    /// happily construct one on a kernel that implements nothing, and the failure only
    /// surfaces later as `EINVAL` from inside `pre_exec`, where it looks like a spawn
    /// error rather than absent confinement.
    ///
    /// So the ABI is queried directly. `ENOSYS` means the syscall does not exist:
    /// the case on Docker Desktop's linuxkit kernel, which does not enable the LSM.
    pub fn new() -> Result<Self, SandboxError> {
        if landlock_abi_version() < 0 {
            return Err(SandboxError::Unavailable {
                platform: "linux",
                detail: "the landlock syscall is not implemented on this kernel \
                         (needs 5.13+ with the LSM enabled)"
                    .into(),
            });
        }

        landlock::Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(AccessFs::from_all(TARGET_ABI))
            .and_then(|r| r.create())
            .map_err(|e| SandboxError::Unavailable {
                platform: "linux",
                detail: format!("landlock ruleset could not be created: {e}"),
            })?;

        Ok(Self)
    }
}

impl Sandbox for LandlockSandbox {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            // Filesystem restrictions are kernel-enforced, but network denial is not
            // implemented yet, so this is deliberately not reported as full kernel
            // confinement.
            level: ConfinementLevel::Partial,
            mechanisms: vec!["landlock"],
            network_denial_enforced: false,
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

        // Network denial is not yet enforceable here, so a policy that requires it
        // must not be silently downgraded.
        if !policy.allow_network {
            return Err(SandboxError::SetupFailed {
                mechanism: "landlock",
                detail: "network denial is not implemented on Linux yet; refusing rather \
                         than reporting confinement that is not applied"
                    .into(),
            });
        }

        let mut command = Command::new(program);
        command.args(args);

        let readable: Vec<_> = policy.readable.clone();
        let writable: Vec<_> = policy.writable.clone();

        // Landlock applies to the calling thread and is inherited across exec, so the
        // ruleset is installed in the child between fork and exec.
        unsafe {
            command.pre_exec(move || {
                use std::io::Error;

                let mut ruleset = landlock::Ruleset::default()
                    .set_compatibility(CompatLevel::BestEffort)
                    .handle_access(AccessFs::from_all(TARGET_ABI))
                    .and_then(|r| r.create())
                    .map_err(|e| Error::other(format!("landlock: {e}")))?;

                if !readable.is_empty() {
                    ruleset = ruleset
                        .add_rules(path_beneath_rules(
                            &readable,
                            AccessFs::from_read(TARGET_ABI),
                        ))
                        .map_err(|e| Error::other(format!("landlock read rules: {e}")))?;
                }

                if !writable.is_empty() {
                    ruleset = ruleset
                        .add_rules(path_beneath_rules(
                            &writable,
                            AccessFs::from_all(TARGET_ABI),
                        ))
                        .map_err(|e| Error::other(format!("landlock write rules: {e}")))?;
                }

                let status = ruleset
                    .restrict_self()
                    .map_err(|e| Error::other(format!("landlock: {e}")))?;

                // Fail closed: if the kernel did not actually enforce the ruleset, do
                // not continue to exec.
                if status.ruleset == RulesetStatus::NotEnforced {
                    return Err(Error::other(
                        "landlock reported the ruleset was not enforced",
                    ));
                }

                Ok(())
            });
        }

        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    /// Landlock is absent on older kernels and in some container runtimes, notably
    /// Docker Desktop's linuxkit kernel, which does not enable the LSM at all.
    ///
    /// Tests needing real enforcement skip there, but set `BRAVEBOT_REQUIRE_LANDLOCK=1` to
    /// turn a skip into a failure. Without that switch a CI run on a kernel lacking
    /// Landlock would report green while never having exercised the sandbox, which is
    /// exactly the false confidence this crate exists to avoid.
    fn sandbox_or_skip() -> Option<LandlockSandbox> {
        match LandlockSandbox::new() {
            Ok(s) => Some(s),
            Err(e) => {
                if std::env::var("BRAVEBOT_REQUIRE_LANDLOCK").as_deref() == Ok("1") {
                    panic!("BRAVEBOT_REQUIRE_LANDLOCK=1 but landlock is unavailable: {e}");
                }
                eprintln!("SKIPPED (landlock unavailable on this kernel): {e}");
                None
            }
        }
    }

    #[test]
    fn capabilities_do_not_overstate_network_denial() {
        let caps = LandlockSandbox.capabilities();
        assert!(
            !caps.network_denial_enforced,
            "network denial is not implemented on Linux yet"
        );
        assert_eq!(caps.level, ConfinementLevel::Partial);
    }

    /// Until network denial is implemented, asking for it must be an error rather than
    /// a sandbox that quietly permits sockets.
    #[test]
    fn a_policy_requiring_network_denial_is_refused() {
        let Some(sandbox) = sandbox_or_skip() else {
            return;
        };
        let err = sandbox
            .command("/bin/true", &[], &SandboxPolicy::strict())
            .expect_err("must refuse rather than under-enforce");
        assert!(matches!(err, SandboxError::SetupFailed { .. }));
    }

    #[test]
    fn a_fully_permissive_policy_is_refused() {
        let policy = SandboxPolicy::strict()
            .allow_network_egress()
            .allow_subprocesses()
            .allow_write("/");
        let err = LandlockSandbox
            .command("/bin/true", &[], &policy)
            .expect_err("must refuse a policy that confines nothing");
        assert!(matches!(err, SandboxError::PolicyTooPermissive));
    }

    #[test]
    fn a_confined_process_runs() {
        let Some(sandbox) = sandbox_or_skip() else {
            return;
        };
        let policy = SandboxPolicy::strict()
            .allow_network_egress()
            .allow_read("/usr")
            .allow_read("/lib")
            .allow_read("/lib64")
            .allow_read("/bin");

        let mut child = sandbox
            .command("/bin/true", &[], &policy)
            .expect("command builds")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("should spawn");
        assert!(child.wait().expect("should wait").success());
    }

    /// The property the backend exists for: writes outside the granted paths fail.
    #[test]
    fn a_confined_process_cannot_write_outside_its_grants() {
        let Some(sandbox) = sandbox_or_skip() else {
            return;
        };
        let policy = SandboxPolicy::strict()
            .allow_network_egress()
            .allow_read("/usr")
            .allow_read("/lib")
            .allow_read("/lib64")
            .allow_read("/bin");

        let target = std::path::Path::new("/tmp/bravebot-landlock-must-not-exist");
        let _ = std::fs::remove_file(target);

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
}
