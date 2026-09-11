//! Linux confinement via Landlock.
//!
//! Filesystem restrictions are applied with Landlock, which needs kernel 5.13 or
//! newer. Availability is probed at runtime rather than assumed: an older kernel, or a
//! container that masks the syscall, means confinement is unavailable and the process
//! is refused instead of run unconfined.
//!
//! Landlock governs the filesystem only. Network denial and subprocess denial need
//! separate mechanisms (e.g. an empty network namespace, or seccomp filtering),
//! which are not implemented yet, so [`Capabilities::network_denial_enforced`] is
//! `false` and the reported level is [`ConfinementLevel::Partial`]. Claiming
//! kernel-level network or subprocess denial here would misreport the guarantee;
//! policies requiring either are refused.

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
    // Landlock has no libc wrapper, so the syscall is issued directly.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
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
            // Filesystem restrictions are kernel-enforced, but network and subprocess
            // denial are not implemented yet, so this is deliberately not reported
            // as full kernel confinement.
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

        // Subprocess denial is not yet enforceable here, so a policy that requires it
        // must not be silently downgraded.
        if !policy.allow_subprocesses {
            return Err(SandboxError::SetupFailed {
                mechanism: "landlock",
                detail: "subprocess denial is not implemented on Linux yet; refusing rather \
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
        // `pre_exec` is unsafe by definition: its closure runs in the forked child, where
        // only async-signal-safe work is allowed.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
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
    use std::io::Write;
    use std::process::Stdio;

    /// cat reporting that the file it was asked for could not be read. Any other code
    /// means it stopped before opening the file, which says nothing about a read grant.
    const CAT_FAILED: i32 = 1;

    /// touch reporting that the write it was asked for failed. Any other code means it
    /// stopped before the write, which says nothing about a write grant.
    const TOUCH_FAILED: i32 = 1;

    /// The narrowest policy a process can actually start under. The loader reads the
    /// binary and the libraries it links, and Landlock has no exemption for that, while
    /// withholding the network or subprocesses is refused outright because neither
    /// denial is enforceable here.
    fn loadable_policy() -> SandboxPolicy {
        SandboxPolicy::strict()
            .allow_network_egress()
            .allow_subprocesses()
            .allow_read("/usr")
            .allow_read("/lib")
            .allow_read("/lib64")
            .allow_read("/bin")
    }

    /// Landlock is absent on kernels before 5.13 and in container runtimes that do not
    /// enable the LSM, notably Docker Desktop's linuxkit kernel.
    ///
    /// A kernel without it fails the tests that need real enforcement rather than
    /// skipping them. Those tests are the whole of what pins confinement on Linux, so a
    /// suite reporting green having never installed a ruleset is the false confidence
    /// this crate exists to avoid, and a kernel that cannot enforce one is worth hearing
    /// about from the test run rather than from the first process that escapes.
    /// `BRAVEBOT_ALLOW_MISSING_LANDLOCK=1` runs the rest of the suite on such a kernel
    /// and says in the output that it did.
    fn sandbox_or_fail() -> Option<LandlockSandbox> {
        match LandlockSandbox::new() {
            Ok(sandbox) => Some(sandbox),
            Err(e) => {
                if std::env::var("BRAVEBOT_ALLOW_MISSING_LANDLOCK").as_deref() == Ok("1") {
                    // Straight at the descriptor rather than through `eprintln!`, which the
                    // test harness captures and replays only for a test that failed. A skip
                    // announced that way is invisible in every run where it is the whole
                    // story, which is a silent skip again by another road.
                    let _ = writeln!(
                        std::io::stderr(),
                        "SKIPPED (BRAVEBOT_ALLOW_MISSING_LANDLOCK=1): {e}"
                    );
                    return None;
                }
                panic!(
                    "landlock is unavailable, so nothing here enforces confinement: {e}. \
                     Set BRAVEBOT_ALLOW_MISSING_LANDLOCK=1 to run the rest of the suite on \
                     a kernel that does not implement it."
                );
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
        let err = LandlockSandbox
            .command("/bin/true", &[], &SandboxPolicy::strict())
            .expect_err("must refuse rather than under-enforce");
        match err {
            SandboxError::SetupFailed { mechanism, detail } => {
                assert_eq!(mechanism, "landlock");
                assert!(
                    detail.contains("network denial"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected SetupFailed for network denial, got: {other:?}"),
        }
    }

    /// Until subprocess denial is implemented, asking for it must be an error rather than
    /// a sandbox that quietly permits fork/exec.
    #[test]
    fn a_policy_requiring_subprocess_denial_is_refused() {
        let err = LandlockSandbox
            .command(
                "/bin/true",
                &[],
                &SandboxPolicy::strict().allow_network_egress(),
            )
            .expect_err("must refuse rather than under-enforce");
        match err {
            SandboxError::SetupFailed { mechanism, detail } => {
                assert_eq!(mechanism, "landlock");
                assert!(
                    detail.contains("subprocess denial"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected SetupFailed for subprocess denial, got: {other:?}"),
        }
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
        let Some(sandbox) = sandbox_or_fail() else {
            return;
        };
        let policy = loadable_policy();

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
        let Some(sandbox) = sandbox_or_fail() else {
            return;
        };
        let policy = loadable_policy();

        // The parent has to be there and empty. Into a directory that does not exist touch
        // fails with ENOENT whatever the ruleset permits, which holds just as well against a
        // sandbox granting every write, and a file left behind by an earlier run would fail
        // every run after it.
        let dir = crate::testutil::scratch_dir("bravebot-landlock-denied-write");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
        let target = dir.join("must-not-exist");

        let mut child = sandbox
            .command("/usr/bin/touch", &[target.display().to_string()], &policy)
            .expect("command builds")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("should spawn");
        let status = child.wait().expect("should wait");

        // touch's own refusal, rather than any failure at all: a process that died before it
        // reached the write exits by signal or with some other code, and neither of those
        // says anything about a grant.
        assert_eq!(
            status.code(),
            Some(TOUCH_FAILED),
            "the write was not what failed"
        );
        assert!(!target.exists(), "file was created despite confinement");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The control that gives every denial here its meaning: without the write rules
    /// installed a write inside the grants fails too, and a test asserting only denials
    /// passes just as readily against a sandbox that permits nothing at all.
    #[test]
    fn a_confined_process_can_write_inside_its_grants() {
        let Some(sandbox) = sandbox_or_fail() else {
            return;
        };

        // The directory has to be there before the ruleset is built: a rule for a path that
        // cannot be opened is dropped rather than refused, so a missing directory is no grant
        // at all and the write below would fail for a reason that proves nothing.
        let dir = crate::testutil::scratch_dir("bravebot-landlock-granted-write");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
        let target = dir.join("written");

        let policy = loadable_policy().allow_write(&dir);
        let mut child = sandbox
            .command("/usr/bin/touch", &[target.display().to_string()], &policy)
            .expect("command builds")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("should spawn");
        assert!(
            child.wait().expect("should wait").success(),
            "a granted write was denied, so nothing else here means anything"
        );
        assert!(target.exists(), "the write succeeded and created nothing");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reads are a grant list as much as writes are: a readable set widened to `/` leaves
    /// every other test here passing while the confined process can read the whole host.
    ///
    /// Both halves run against the same file, because a cat that failed for reasons of
    /// its own exits exactly as one refused the file does.
    #[test]
    fn a_confined_process_cannot_read_outside_its_grants() {
        let Some(sandbox) = sandbox_or_fail() else {
            return;
        };

        let dir = crate::testutil::scratch_dir("bravebot-landlock-denied-read");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
        let target = dir.join("readable");
        std::fs::write(&target, b"contents").expect("the file is writable");

        let cat = |policy: &SandboxPolicy| {
            sandbox
                .command("/usr/bin/cat", &[target.display().to_string()], policy)
                .expect("command builds")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("should spawn")
        };

        let mut granted = cat(&loadable_policy().allow_read(&dir));
        assert_eq!(
            granted.wait().expect("should wait").code(),
            Some(0),
            "a granted read failed, so nothing below means anything"
        );

        let mut refused = cat(&loadable_policy());
        assert_eq!(
            refused.wait().expect("should wait").code(),
            Some(CAT_FAILED),
            "the read was not what failed"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
