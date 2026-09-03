BINARY = bravebot
VERSION = $(shell sed -nE 's/^version[[:space:]]*=[[:space:]]*"([0-9]+\.[0-9]+\.[0-9]+)".*/\1/p' Cargo.toml | head -n 1)
TAG = v$(VERSION)

# Forwarded into the cross-build container, which does not inherit the host environment.
BUILD_ENV = SERVICES_KEY_AICHAT BRAVE_SERVICES_KEY_ID BRAVE_AI_CHAT_ENDPOINT \
            BRAVE_AI_CHAT_PREMIUM_ENDPOINT BRAVE_AI_CHAT_DEFAULT_MODEL \
            BRAVEBOT_ALLOW_UNCONFIGURED_BUILD

.PHONY: help
help:
	@echo "bravebot $(VERSION)"
	@echo
	@echo "Development:"
	@echo "  make init           Configure bravebot, Claude Code and Codex; install hooks"
	@echo "  make hooks          Point git at the checked-in pre-commit hook"
	@echo "  make build          Debug build"
	@echo "  make test           Run all tests"
	@echo "  make check          Format check, clippy, and tests"
	@echo "  make check-spec     Check docs/specs against the implementation"
	@echo "  make locales        What each translation has, and what it is missing"
	@echo "  make check-linux    The same checks on Linux, current stable toolchain"
	@echo "  make fmt            Apply formatting"
	@echo "  make aws-logout     End every cached AWS SSO session, to test signing in again"
	@echo
	@echo "Reproducible cross-builds (requires Docker):"
	@echo "  make all-platforms  Every target below"
	@echo "  make darwin-arm64   macOS Apple silicon"
	@echo "  make darwin-amd64   macOS Intel"
	@echo "  make linux-amd64    Linux x86_64"
	@echo "  make linux-arm64    Linux aarch64"
	@echo "  make windows-amd64  Windows x86_64"
	@echo "  make windows-arm64  Windows aarch64"
	@echo
	@echo "Releasing:"
	@echo "  make bump-version BUMP=bugfix|minor|major   Set the next version"
	@echo "  make github-release                         Tag it and let CI publish"
	@echo
	@echo "  make clean          Remove build output"

# agents/ is the checked-in source of truth for skills and AGENTS.md, and no tool reads
# it: each tool looks under its own discovery paths. This creates the symlinks that
# bridge them. The links are gitignored, so a fresh clone needs it once, and it is
# idempotent, so re-running costs nothing.
.PHONY: init
init: hooks
	python3 agents/setup.py link

# .git/hooks is not versioned, so a fresh clone commits with nothing checking it until this
# runs. Idempotent, and part of `init` so nobody has to know it exists.
.PHONY: hooks
hooks:
	git config core.hooksPath .githooks

.PHONY: build
build:
	cargo build

.PHONY: test
test:
	cargo test --all

.PHONY: fmt
fmt:
	cargo fmt --all

# The counterpart to the sign-in a Bedrock turn does for itself, for testing that path deliberately
# rather than waiting for a token to expire.
#
# Every profile, because that is the only thing the CLI offers: `aws sso logout` removes every cached
# token and takes no option to narrow it, so `--profile` would scope nothing while reading as though
# it had. One token serves every profile sharing an sso_session, and other tools reading the same
# cache, Claude Code among them, need a fresh `aws sso login` after this.
.PHONY: aws-logout
aws-logout:
	@echo "ending every cached AWS SSO session; sign in again with: aws sso login --profile <name>"
	aws sso logout

# Everything CI enforces, runnable locally before pushing.
.PHONY: check
check:
	cargo fmt --all -- --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --all

# The mechanical half of the spec check: clause numbering, the tests each clause names,
# the paths it governs, the symbols it guards, and the table in the specs README. No model
# is involved, so this is deterministic and belongs in CI. Whether the code actually does
# what a clause says needs a reading of the governed source: run the check-spec skill for
# that half.
.PHONY: check-spec
check-spec:
	python3 agents/skills/check-spec/selftest.py
	python3 agents/skills/check-spec/check-spec.py --mechanical-only

# What each catalog has of the reference, and what it is missing. The build says so too, in a
# warning, but a warning is only printed when the build script actually runs, so a translator
# working through a file learns nothing from a cached build. This always answers.
.PHONY: locales
locales:
	@ref=crates/i18n/locales/en-US.ftl; \
	ids() { grep -oE '^[a-z][a-z0-9-]+ =' "$$1" | tr -d ' ='; }; \
	total=$$(ids $$ref | wc -l | tr -d ' '); \
	echo "en-US  $$total messages, the reference"; \
	for f in crates/i18n/locales/*.ftl; do \
		case "$$f" in *en-US.ftl) continue;; esac; \
		tag=$$(basename "$$f" .ftl); \
		have=$$({ ids $$ref; ids "$$f"; } | sort | uniq -d | wc -l | tr -d ' '); \
		echo "$$tag  $$have of $$total"; \
		{ ids $$ref; ids "$$f"; } | sort | uniq -u | sed 's/^/    missing: /'; \
	done

# Runs the same checks on Linux with the current stable toolchain. Worth doing before
# pushing platform-specific code: a macOS host never compiles the Linux backend, and
# clippy gains lints between releases, so both can fail in CI while passing locally.
.PHONY: check-linux
check-linux:
	docker run --rm --platform linux/amd64 -v "$(PWD):/src:ro" -w /work rust:slim sh -c '\
		cp -r /src/. /work && \
		rustup component add clippy rustfmt >/dev/null 2>&1 && \
		cargo fmt --all -- --check && \
		cargo clippy --all-targets --all-features -- -D warnings && \
		cargo test --all'

.PHONY: darwin-arm64
darwin-arm64:
	$(call cross-build,$@,aarch64-apple-darwin)

.PHONY: darwin-amd64
darwin-amd64:
	$(call cross-build,$@,x86_64-apple-darwin)

.PHONY: linux-amd64
linux-amd64:
	$(call cross-build,$@,x86_64-unknown-linux-gnu)

.PHONY: linux-arm64
linux-arm64:
	$(call cross-build,$@,aarch64-unknown-linux-gnu)

.PHONY: windows-amd64
windows-amd64:
	$(call cross-build,$@,x86_64-pc-windows-gnu)

.PHONY: windows-arm64
windows-arm64:
	$(call cross-build,$@,aarch64-pc-windows-gnullvm)

.PHONY: all-platforms
all-platforms: darwin-arm64 darwin-amd64 linux-amd64 linux-arm64 windows-amd64 windows-arm64
	@echo
	@echo "built:"
	@ls -1 dist/

# Symbols are kept during the build because Rust's own strip can corrupt some targets
# under zigbuild, so they are removed here instead.
#
# rust-objcopy is LLVM-based and handles Mach-O, ELF, and PE alike, so one tool covers
# every target; the per-target GNU strip binaries are not all present in the image.
RUST_LIB_DIR = /usr/local/rustup/toolchains/1.93.0-x86_64-unknown-linux-gnu/lib
STRIP_TOOL = $(RUST_LIB_DIR)/rustlib/x86_64-unknown-linux-gnu/bin/rust-objcopy
.PHONY: strip
strip:
	@for f in dist/$(BINARY)-*; do \
		case "$$f" in *.sha256|*SHA256SUMS) continue;; esac; \
		docker run --rm -v "$(PWD)/dist:/dist" \
			-e LD_LIBRARY_PATH=$(RUST_LIB_DIR) \
			ghcr.io/rust-cross/cargo-zigbuild:0.23.0 \
			$(STRIP_TOOL) --strip-all "/dist/$$(basename $$f)"; \
	done
	@echo "stripped:"
	@ls -lh dist/ | awk 'NR>1 {print "  " $$9, $$5}'

.PHONY: checksums
checksums:
	@cd dist && rm -f ./*.sha256 SHA256SUMS && \
	for f in $(BINARY)-*; do \
		shasum -a 256 "$$f" | awk '{print $$1}' > "$$f.sha256"; \
		shasum -a 256 "$$f" >> SHA256SUMS; \
	done
	@echo "wrote dist/SHA256SUMS"

# Edits the version in place and stops. Committing it is a separate, reviewable step,
# and `github-release` refuses to tag a dirty tree, so the bump cannot ride along
# untracked with a tag that claims to name it.
.PHONY: bump-version
bump-version:
	@if [ "$(BUMP)" != "bugfix" ] && [ "$(BUMP)" != "minor" ] && [ "$(BUMP)" != "major" ]; then \
		echo "error: BUMP must be one of: bugfix, minor, major"; \
		exit 1; \
	fi
	@set -eu; \
	current="$(VERSION)"; \
	if [ -z "$$current" ]; then \
		echo "error: unable to read version from Cargo.toml"; \
		exit 1; \
	fi; \
	major="$${current%%.*}"; rest="$${current#*.}"; \
	minor="$${rest%%.*}"; patch="$${rest#*.}"; \
	case "$(BUMP)" in \
		bugfix) patch="$$((patch + 1))" ;; \
		minor) minor="$$((minor + 1))"; patch=0 ;; \
		major) major="$$((major + 1))"; minor=0; patch=0 ;; \
	esac; \
	next="$$major.$$minor.$$patch"; \
	awk -v v="$$next" ' \
		BEGIN { in_pkg = 0; done = 0 } \
		/^\[/ { in_pkg = ($$0 == "[workspace.package]") } \
		in_pkg && !done && /^version[[:space:]]*=/ { \
			print "version = \"" v "\""; done = 1; next \
		} \
		{ print }' Cargo.toml > Cargo.toml.tmp; \
	mv Cargo.toml.tmp Cargo.toml; \
	if [ "$$(sed -nE 's/^version[[:space:]]*=[[:space:]]*"([0-9]+\.[0-9]+\.[0-9]+)".*/\1/p' Cargo.toml | head -n 1)" != "$$next" ]; then \
		echo "error: Cargo.toml version was not rewritten"; \
		exit 1; \
	fi; \
	cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null; \
	V="$$next" node -e ' \
const fs = require("node:fs"); \
const pkg = JSON.parse(fs.readFileSync("package.json", "utf8")); \
if (!process.env.V) { throw new Error("version not passed through"); } \
pkg.version = process.env.V; \
fs.writeFileSync("package.json", JSON.stringify(pkg, null, 2) + "\n");'; \
	echo "bumped $$current -> $$next (Cargo.toml, Cargo.lock, package.json)"; \
	echo "commit this, then run: make github-release"

# Tags the current version and pushes it. The tag push is the only thing that triggers
# a release build, so everything CI needs must already be committed and on origin.
.PHONY: github-release
github-release:
	@set -eu; \
	if [ -z "$(VERSION)" ]; then \
		echo "error: unable to read version from Cargo.toml"; \
		exit 1; \
	fi; \
	if [ "$$(node -p 'require("./package.json").version')" != "$(VERSION)" ]; then \
		echo "error: package.json version does not match Cargo.toml ($(VERSION)); run make bump-version"; \
		exit 1; \
	fi; \
	if ! git diff --quiet || ! git diff --cached --quiet; then \
		echo "error: working tree must be clean before tagging"; \
		exit 1; \
	fi; \
	branch="$$(git rev-parse --abbrev-ref HEAD)"; \
	if [ "$$branch" != "main" ]; then \
		echo "error: releases are tagged from main, not $$branch"; \
		exit 1; \
	fi; \
	git fetch --quiet origin main; \
	if [ "$$(git rev-parse HEAD)" != "$$(git rev-parse origin/main)" ]; then \
		echo "error: HEAD differs from origin/main; push or pull first"; \
		exit 1; \
	fi; \
	if git rev-parse -q --verify "refs/tags/$(TAG)" >/dev/null; then \
		echo "error: tag $(TAG) already exists"; \
		exit 1; \
	fi; \
	git tag -a -m "bravebot $(TAG)" "$(TAG)"; \
	git push origin "$(TAG)"; \
	echo "pushed $(TAG); CI will build, checksum, and publish the GitHub release"; \
	echo "watch with: gh run watch --repo brave-experiments/brave-bot"

.PHONY: clean
clean:
	cargo clean
	rm -rf dist

# Configuration reaches the build as a BuildKit secret rather than a build argument,
# which would record the signing key in the image metadata. The temporary file is
# mode 600 and removed even if the build fails.
define cross-build
	set -e; \
	env_file="$$(mktemp)"; trap 'rm -f "$$env_file"' EXIT INT TERM; \
	for name in $(BUILD_ENV); do \
		eval "value=\$$$$name"; \
		if [ -n "$$value" ]; then printf 'export %s=%s\n' "$$name" "$$value" >> "$$env_file"; fi; \
	done; \
	DOCKER_BUILDKIT=1 docker build -f Dockerfile.cross -t $(BINARY)-$(1) \
		--build-arg TARGET=$(2) \
		--secret id=bravebot_env,src="$$env_file" .
	$(call extract,$(BINARY)-$(1),$(1))
endef

# `docker create` on a scratch image needs a command argument even though it never
# runs; the container exists only so the binary can be copied out.
define extract
	mkdir -p dist
	docker rm -f tmp-$(BINARY)-$(2) 2>/dev/null || true
	docker create --name tmp-$(BINARY)-$(2) $(1) /dev/null
	docker cp tmp-$(BINARY)-$(2):/$(BINARY) dist/$(call artifact,$(2))
	docker rm tmp-$(BINARY)-$(2)
endef

define artifact
$(BINARY)-$(1)$(if $(findstring windows,$(1)),.exe,)
endef
