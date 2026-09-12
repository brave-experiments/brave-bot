# Releasing

Reproducible cross-platform binaries are built in a pinned container, so the same artifact
comes out on any host:

```sh
make all-platforms
```

## Publishing a version

Two steps to name the version, then two manual publishes: Jenkins for signed binaries, then
GitHub Actions for the npm package. Either can wait days after the tag.

```sh
make bump-version BUMP=bugfix   # or minor, major
# review the commit it made, land it on main
make github-release
# later: Jenkins job bravebot-build with UPLOAD and RELEASE
# later: Actions → Publish npm, with tag v<version>
```

`bump-version` rewrites the version in `Cargo.toml`, `Cargo.lock`, `package.json`, and
`package-lock.json`, commits exactly those four as `Bump version to <version>`, and stops there:
nothing is pushed and nothing is tagged. It refuses if any of the four is already modified,
rather than committing changes it did not write. `github-release` refuses to tag unless the tree
is clean, the two version files agree, and HEAD is `main` at the release remote's `main`, then
pushes `v<version>` there.

**The release remote.** `origin` by default, which is right for a clone of this repository and
wrong for a clone of a fork of it, where `origin` names the fork. A tag pushed to a fork is one
the releases page never sees, and the tag ruleset here refuses an update or a deletion, so the
mistake cannot be corrected in place: the version has to be bumped again. A clone whose `origin`
is a fork names the remote that is not, once:

```sh
git config bravebot.releaseRemote upstream
```

`github-release` fails before tagging if no remote by that name exists.

The tag push does not publish binaries. GitHub Actions still builds and tests on the tag.
Signed, configured assets are built, notarised, and uploaded by the Jenkins job
`bravebot-build` with `UPLOAD` and `RELEASE` enabled. That job builds the tip of the branch it
is given. A RELEASE run refuses unless tag `v<version>` from `Cargo.toml` already exists and
points at that commit, then creates a GitHub release of that name and attaches the signed
binaries plus the `.sha256` files written after signing. `gh release create` fails if that
release already exists.

After Jenkins has created that GitHub release, dispatch `publish-npm.yml` with the same tag to
publish `@brave/bravebot`. OIDC trusted publishing; there is no `NPM_TOKEN`. The trusted
publisher on npmjs.com must name that workflow file. Run the workflow from the version tag
(Use workflow from / `--ref`). The npm `postinstall` verifies the binary against its published
`.sha256` before writing it.

`BRAVEBOT_ALLOW_UNCONFIGURED_BUILD` is set in GitHub Actions so forks and PRs compile without
secrets. Jenkins does not set it on an upload, so a missing credential fails the release
instead of shipping a binary that cannot reach the backend.

Darwin binaries are codesigned and notarised, Windows binaries are Authenticode-signed.
