---
id: SANDBOX
title: Confining subprocesses
status: normative
governs:
  - crates/sandbox/src/lib.rs
  - crates/sandbox/src/policy.rs
  - crates/sandbox/src/linux.rs
  - crates/sandbox/src/macos.rs
---

## Scope

Operating-system confinement for processes that run code we did not write, which today means the
stdio servers in [mcp.md](mcp.md). What this is *not* for is the rest of the system: a processor is
a model call made by our own code, and a program the user asked for runs with the access their own
shell would give it. Confining our own code would fence in the trusted half and leave the untrusted
half free.

Confinement is an operating-system boundary. Everywhere else in these specs the boundary is the
capability set and the label on a value, which is a different mechanism answering a different
question.

## Clauses

### SANDBOX-1: confinement fails closed

If confinement cannot be established the process does not run. An unavailable backend refuses to
spawn rather than falling back, and the platform lookup never hands back a backend that would
confine nothing.

**Why.** Silently degrading is worse than an error: the caller believes it has a guarantee it does
not have, and the audit trail records a sandbox that was never applied.

`verified-by: bravebot_sandbox::lib::an_unavailable_backend_refuses_to_spawn`
`verified-by: bravebot_sandbox::lib::refusal_is_not_a_silent_fallback`
`verified-by: bravebot_sandbox::lib::the_platform_lookup_never_returns_an_unconfined_backend`
`verified-by: bravebot_sandbox::lib::errors_explain_the_refusal`

### SANDBOX-2: a policy that would confine nothing is refused

A profile starts denying everything, grants accumulate onto it, and a fully permissive policy is
rejected rather than applied. Granting everything is not a confinement decision.

`verified-by: bravebot_sandbox::policy::strict_permits_nothing`
`verified-by: bravebot_sandbox::policy::allowances_accumulate`
`verified-by: bravebot_sandbox::policy::a_strict_policy_is_meaningful`
`verified-by: bravebot_sandbox::policy::granting_everything_is_not_meaningful`
`verified-by: bravebot_sandbox::policy::network_alone_remains_meaningful`
`verified-by: bravebot_sandbox::macos::a_fully_permissive_policy_is_refused`
`verified-by: bravebot_sandbox::linux::a_fully_permissive_policy_is_refused`

### SANDBOX-3: the network is denied unless it was asked for

A confined process reaches neither the network nor the filesystem outside its grants.

`verified-by: bravebot_sandbox::macos::network_is_only_allowed_when_requested`
`verified-by: bravebot_sandbox::macos::a_confined_process_cannot_reach_the_network`
`verified-by: bravebot_sandbox::macos::a_confined_process_cannot_write_outside_its_grants`
`verified-by: bravebot_sandbox::macos::a_confined_process_runs`

### SANDBOX-4: a path cannot inject profile syntax

Grants are paths, and a path is content: one containing profile syntax is quoted rather than
interpreted, and a profile built from a hostile path still applies.

**Why.** A grant list assembled from paths would otherwise be a place where a filename decides what
the sandbox permits.

`verified-by: bravebot_sandbox::macos::paths_cannot_inject_profile_syntax`
`verified-by: bravebot_sandbox::macos::a_profile_containing_a_hostile_path_still_applies`
`verified-by: bravebot_sandbox::macos::granted_paths_appear_as_subpath_rules`

### SANDBOX-5: capabilities report what the kernel actually enforces, and never more

A backend says what it can enforce rather than what it was asked for, and a policy demanding
something the backend cannot deliver is refused. `bravebot doctor` reports the level in force.

**Why.** An overstated capability is the same failure as a silent fallback, reached by a different
road.

`verified-by: bravebot_sandbox::macos::capabilities_report_kernel_enforcement`
`verified-by: bravebot_sandbox::linux::capabilities_do_not_overstate_network_denial`
`verified-by: bravebot_sandbox::linux::a_policy_requiring_network_denial_is_refused`
`verified-by: bravebot_sandbox::lib::an_unavailable_backend_reports_no_confinement`
`verified-by: bravebot_sandbox::policy::confinement_levels_render_for_the_audit_trail`
