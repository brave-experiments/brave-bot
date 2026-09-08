---
id: RELEASE
title: Releases
status: normative
governs:
  - Makefile
  - .github/workflows/ci.yml
  - .github/workflows/publish-npm.yml
  - npm/scripts/postinstall.js
  - package.json
  - package-lock.json
---

## Scope

Turning a commit into binaries somebody else installs: what names a version, what starts a
release, what refuses one, and what an installer trusts about what it fetched.

Building for a platform is not this topic. Reproducible cross-builds are ordinary code, described
in [../development.md](../development.md). This file governs only the path from a version to a
published asset, and the checks along it.

GitHub Actions compiles and tests. It does not create a GitHub release. Signed, configured
binaries are built and published by the Jenkins job `brave-bot-build` in the devops repository.
The npm package is published afterwards, from a manually dispatched GitHub Actions workflow,
once that release exists.

Nothing here is pinned by a Rust test. The rules below are enforced by a refusal in the tagging
path, the Jenkins publish path, or the npm publish workflow rather than by anything the test
suite can execute, so each clause says in brackets what makes it hold.

## Clauses

<a id="RELEASE-1"></a>
### RELEASE-1: one version names a release, and every file that states it agrees

The workspace manifest holds the version. Every other file that repeats it, the npm package
manifest in particular, states the same value, and a disagreement stops a release rather than
being resolved in favour of either.

**Why.** The installer derives the tag it downloads from the version it was published with, so two
files disagreeing does not produce a mislabelled release, it produces a release whose assets the
installer looks for under a name that was never uploaded.

`verified-by: by-construction (bumping rewrites every file that states the version in one step, and both tagging and Jenkins refuse a mismatch)`

<a id="RELEASE-2"></a>
### RELEASE-2: setting the next version publishes nothing

Choosing a version edits the tree and stops. It commits nothing, pushes nothing, and tags
nothing.

**Why.** A version bump is a reviewable change, and the review is worth having: it is the last
point where the size of a release can be questioned. Bundling the bump into the act of releasing
removes that point, and makes a mistyped bump irreversible in the same breath.

`verified-by: by-construction (the bump target writes files and prints the next step, and contains no git command)`

<a id="RELEASE-3"></a>
### RELEASE-3: GitHub Actions does not publish a GitHub release

No branch push, no pull request, no tag push, and no manually dispatched GitHub Actions run
creates a GitHub release or attaches binaries people install. Publication of those assets
happens in Jenkins, when `brave-bot-build` is run with `UPLOAD` and `RELEASE`. The npm package
is a later step, specified below, and is not a GitHub release.

**Why.** GitHub Actions cannot codesign Darwin or Authenticode-sign Windows. If it published,
the public assets would be unsigned until a later overwrite, and a tag re-push would put those
unsigned bytes back.

`verified-by: by-construction (ci.yml has no release job and contents: write is not granted)`

<a id="RELEASE-4"></a>
### RELEASE-4: nothing is tagged from a tree that was not reviewed

A tag is created only from a clean working tree, on the trunk, at the same commit the remote
trunk is at, and only when that tag does not already exist.

**Why.** The tag is the record of what was released, so it has to name a commit others can see.
Tagging a dirty tree names a commit that does not contain what was built; tagging a branch names
work that was never reviewed; tagging ahead of the remote names a commit nobody else has.

`verified-by: by-construction (each condition is a separate refusal in the tagging path, checked before the tag is created)`

<a id="RELEASE-5"></a>
### RELEASE-5: the published name is the version in the tree that was built

The makefile names the tag `v` plus the version in `Cargo.toml`, after refusing a
`package.json` mismatch. Jenkins names the GitHub release the same way from the `Cargo.toml`
of the commit it checked out, after the same refusal. Neither path publishes under a name that
disagrees with the tree it built.

**Why.** The installer derives the tag it downloads from the version it was published with, so
a release whose name is not the version in the tree is a release whose assets the installer
looks for under a name that was never uploaded.

`verified-by: by-construction (the tagging path sets the tag from Cargo.toml after checking package.json, a Jenkins RELEASE refuses unless that tag already names the commit it checked out and package.json states the same version, then names the GitHub release from Cargo.toml, and the npm publish refuses unless the tag is v plus the same version in both files)`

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

`verified-by: by-construction (the build fails on missing configuration unless permitted, and a Jenkins upload does not grant that permission)`

<a id="RELEASE-7"></a>
### RELEASE-7: a release has every supported platform in it, or is not published

Every platform the installer can ask for is present before anything is uploaded. A missing one
fails the release instead of publishing the rest.

**Why.** The installer maps a platform to exactly one asset name, so a partial release is not a
smaller release. It is a broken install for whoever is on the platform that went missing, and it
reads to them as the release existing but the tool not working.

`verified-by: by-construction (the publish job in devops downloads each of the six named assets before it creates the GitHub release; a missing object fails that download. This tree does not contain that job)`

<a id="RELEASE-8"></a>
### RELEASE-8: every asset is published with a checksum of the bytes that were uploaded

Each asset has a checksum published beside it, covering the asset in its final form, after every
step that alters its bytes.

**Why.** A checksum taken before a later step rewrites the file is worse than none: it will not
match, and the mismatch looks exactly like tampering, so the one signal that is supposed to
distinguish a bad download from a good one now fires on every good one.

`verified-by: by-construction (Jenkins attaches the .sha256 written after signing, covering the bytes that were uploaded)`

<a id="RELEASE-9"></a>
### RELEASE-9: a downloaded binary is checked against its published checksum before it is installed

The installer fetches the published checksum, compares it against what it downloaded, and writes
no executable when the two differ or the checksum is not well formed. Well formed is the digest
and nothing else: sixty-four hex digits, with no filename beside them.

**Why.** Without this the binary runs on the strength of the transport alone, and a substituted
release asset is indistinguishable from a good one. Signing proves who produced the Darwin and
Windows binaries; the checksum is what the installer can check on every platform, including
Linux.

`verified-by: by-construction (the install step hashes what it downloaded, compares it against the published value, and exits without writing on a mismatch)`

<a id="RELEASE-10"></a>
### RELEASE-10: the npm package is published by hand, after that version's GitHub release exists

A branch push, a pull request, and a tag push do not publish to the registry. Someone dispatches
`publish-npm.yml` with the version tag after Jenkins has created the GitHub release of that name.
A dispatch whose tag is not `v` plus the version in the tree, or whose GitHub release is missing
any platform asset or checksum, is refused rather than publishing a wrapper whose installer has
nothing to fetch.

**Why.** The installer derives the download from the version it was published with. Publishing
the wrapper first makes every install fail until the assets exist, which reads as the tool being
broken. Jenkins is started by hand, days later if need be, so npm publish is the same kind of
step: it happens when somebody chooses, not when the tag lands. Publishing from a branch puts
an unreviewed version on the registry.

`verified-by: by-construction (publish-npm.yml runs only on workflow_dispatch, checks out the given tag, refuses unless that tag is v plus the version in both files, and refuses unless each named asset and its checksum are on the GitHub release)`

<a id="RELEASE-11"></a>
### RELEASE-11: the registry authenticates the workflow, not a stored token

The publish job proves itself with a short-lived OIDC identity for this workflow in this
repository, and sets no long-lived npm credential. The package it publishes carries provenance
for that run.

**Why.** A token in GitHub secrets is a credential that publishes if it leaks, and it outlives
the run that needed it. OIDC binds the publish to this file on this repository, so a different
workflow, or the same workflow in a fork, cannot use it.

`verified-by: by-construction (the publish job grants id-token: write, sets no NPM_TOKEN or NODE_AUTH_TOKEN, and calls npm publish --access public --provenance)`

<a id="RELEASE-12"></a>
### RELEASE-12: the npm lockfile is committed, CI installs from it, and it is linted

An install in CI uses the committed lockfile rather than resolving anew, and a lockfile that
pulls from somewhere other than the npm registry, or over http, fails that job.

**Why.** The published tarball is this repository's wrapper scripts. A dependency that appeared
only at install time would be bytes nobody reviewed, and the first place that would show up is
the pipeline that publishes.

`verified-by: by-construction (package-lock.json is in the tree, both workflows run npm ci --ignore-scripts, and lockfile-lint refuses hosts other than npm and non-https URLs)`

## Known costs

- **Nothing here is pinned by a test.** Every clause is by-construction, which means a refusal can
  be removed and only a reader will notice. The tagging path is shell in a makefile, publication
  is a Jenkins job in another repository, and a test that shelled out to a real tag push would
  have to publish something to prove anything.

- **Publication lives outside this repository.** `brave-bot-build` in devops is what signs and
  attaches assets. A change there can break RELEASE-6 through RELEASE-8 without this tree
  noticing. RELEASE-9 is the installer in this repository: a checksum Jenkins ships in the
  wrong form is refused here, not accepted.

- **The refusals before a tag guard the tag, not the branch.** Tagging checks the branch, the
  remote, and the agreement between the two version files. The publish job builds the tip of a
  branch it is handed, refuses a `package.json` mismatch, and refuses unless tag `v` plus the
  version in `Cargo.toml` already names that commit. A commit that is not `origin/main` can
  still be published if that is what the tag names and BRANCH points there.

- **Who published a release is not recoverable from here.** A publish is a parameterised build in
  another system, so who asked for it lives in that system's history. The job requires the tag
  to name the commit it built, so a GitHub release created by that job is built from that tag. A
  tag with no GitHub release is not a publish.

- **No build in this repository is configured.** Every job in the workflow carries the permission
  to compile without credentials, so the first configured build of a commit happens in the publish
  job. A change that only fails with configuration baked in gets no signal before then.

- **A second Jenkins run of RELEASE for the same version fails.** `gh release create` does not
  replace an existing release, so a retry after a successful publish, or after a hand-made
  release of the same tag, stops before upload.

- **The trusted publisher is configured on npmjs.com, not here.** OIDC will refuse until that
  record names this repository and `publish-npm.yml` exactly. A mismatch looks like a 404 from
  the registry. Enabling 2FA on maintainer npm accounts is also outside this tree.

- **A second npm publish of the same version fails at the registry.** Dispatching before Jenkins
  has created the GitHub release fails the asset check instead. A delayed Jenkins run does not
  start npm publish on its own.

- **OIDC trusted publishing does not support Jenkins.** That is why this one publication step
  is in GitHub Actions while the signed binaries stay in Jenkins.
