---
id: RELEASE
title: Releases
status: normative
governs:
  - Makefile
  - .github/workflows/ci.yml
  - npm/scripts/postinstall.js
  - package.json
---

## Scope

Turning a commit into binaries somebody else installs: what names a version, what starts a
release, what refuses one, and what an installer trusts about what it fetched.

Building for a platform is not this topic. Reproducible cross-builds are ordinary code, described
in [../development.md](../development.md). This file governs only the path from a version to a
published asset, and the checks along it.

Nothing here is pinned by a Rust test. A release runs in CI against a pushed tag, and the rules
below are enforced by a refusal in the release path rather than by anything the test suite can
execute, so each clause says in brackets what makes it hold.

## Clauses

<a id="RELEASE-1"></a>
### RELEASE-1: one version names a release, and every file that states it agrees

The workspace manifest holds the version. Every other file that repeats it, the npm package
manifest in particular, states the same value, and a disagreement stops a release rather than
being resolved in favour of either.

**Why.** The installer derives the tag it downloads from the version it was published with, so two
files disagreeing does not produce a mislabelled release, it produces a release whose assets the
installer looks for under a name that was never uploaded.

`verified-by: by-construction (bumping rewrites every file that states the version in one step, and both tagging and the release job refuse a mismatch)`

<a id="RELEASE-2"></a>
### RELEASE-2: setting the next version publishes nothing

Choosing a version edits the tree and stops. It commits nothing, pushes nothing, and tags
nothing.

**Why.** A version bump is a reviewable change, and the review is worth having: it is the last
point where the size of a release can be questioned. Bundling the bump into the act of releasing
removes that point, and makes a mistyped bump irreversible in the same breath.

`verified-by: by-construction (the bump target writes files and prints the next step, and contains no git command)`

<a id="RELEASE-3"></a>
### RELEASE-3: a pushed version tag is the only thing that publishes

No branch push, no pull request, and no manually dispatched run produces a release. Publication
happens for a pushed tag naming a version, and for nothing else.

**Why.** One trigger is one thing to reason about when asking whether something was published.
A second path, a dispatch button in particular, means the answer depends on who pressed what,
which is not recoverable from the repository afterwards.

`verified-by: by-construction (the release job runs only for a ref under refs/tags/v, and no other trigger reaches it)`

<a id="RELEASE-4"></a>
### RELEASE-4: nothing is tagged from a tree that was not reviewed

A tag is created only from a clean working tree, on the trunk, at the same commit the remote
trunk is at, and only when that tag does not already exist.

**Why.** The tag is the record of what was released, so it has to name a commit others can see.
Tagging a dirty tree names a commit that does not contain what was built; tagging a branch names
work that was never reviewed; tagging ahead of the remote names a commit nobody else has.

`verified-by: by-construction (each condition is a separate refusal in the tagging path, checked before the tag is created)`

<a id="RELEASE-5"></a>
### RELEASE-5: a tag that disagrees with the tree it points at publishes nothing

When the version a tag names is not the version in the commit it points at, the release fails
and no asset is uploaded.

**Why.** A tag can be created by hand, so the tagging refusals are not the only way one arrives.
This is the check that does not depend on how the tag was made.

**Note.** This is the same agreement RELEASE-1 requires, checked at a different moment: once
before the tag exists, once after it does.

`verified-by: by-construction (the release job compares the tag name against the version in the checked-out tree before uploading anything)`

<a id="RELEASE-6"></a>
### RELEASE-6: a published binary carries the configuration it needs to run

A build that is going to be published fails when the configuration baked into it is missing.
Builds that nobody installs, a pull request or a fork in particular, are allowed to lack it and
compile anyway.

**Why.** Configuration is captured at build time, so a binary built without it cannot reach the
backend at all. Shipping one moves the failure from a build log, where it is one person's problem
and obvious, to every machine that installed it, where it reads as the program being broken.

**Note.** The permission to build unconfigured is granted by an exact value, so a setting that is
present but means nothing does not grant it.

`verified-by: by-construction (the build fails on missing configuration unless permitted, and a tag build does not grant that permission)`

<a id="RELEASE-7"></a>
### RELEASE-7: a release has every supported platform in it, or is not published

Every platform the installer can ask for is present before anything is uploaded. A missing one
fails the release instead of publishing the rest.

**Why.** The installer maps a platform to exactly one asset name, so a partial release is not a
smaller release. It is a broken install for whoever is on the platform that went missing, and it
reads to them as the release existing but the tool not working.

`verified-by: by-construction (the release job lists the expected assets and fails when one is absent)`

<a id="RELEASE-8"></a>
### RELEASE-8: every asset is published with a checksum of the bytes that were uploaded

Each asset has a checksum published beside it, covering the asset in its final form, after every
step that alters its bytes.

**Why.** A checksum taken before a later step rewrites the file is worse than none: it will not
match, and the mismatch looks exactly like tampering, so the one signal that is supposed to
distinguish a bad download from a good one now fires on every good one.

`verified-by: by-construction (checksums are computed in the release job after the binaries are stripped and immediately before upload)`

<a id="RELEASE-9"></a>
### RELEASE-9: a downloaded binary is checked against its published checksum before it is installed

The installer fetches the published checksum, compares it against what it downloaded, and writes
no executable when the two differ or the checksum is not well formed.

**Why.** Without this the binary runs on the strength of the transport alone, and a substituted
release asset is indistinguishable from a good one. The assets are not signed, so this is the only
thing standing between a replaced download and an executable on the user's machine.

`verified-by: by-construction (the install step hashes what it downloaded, compares it against the published value, and exits without writing on a mismatch)`

## Known costs

- **Nothing here is pinned by a test.** Every clause is by-construction, which means a refusal can
  be removed and only a reader will notice. The release path is shell in a makefile and a workflow,
  neither of which the Rust test suite can reach, and a test that shelled out to a real tag push
  would have to publish something to prove anything.

- **Released assets are not signed or notarised.** macOS refuses a downloaded binary that carries
  no signature, so a user who installs one by hand has to clear it themselves. RELEASE-9 is what
  makes an unsigned asset safe to fetch through the installer, and it is a weaker guarantee: it
  proves the bytes match what was published, not who published them. Anyone who can write to the
  release can write both the asset and its checksum.

- **The npm package is not published.** RELEASE-1 and RELEASE-5 already require the npm manifest to
  agree with the tag, and the installer already resolves assets from a published release, so the
  packaging half is specified and exercised while publication is not. What is unproven is the
  registry step itself.
