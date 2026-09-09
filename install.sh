#!/usr/bin/env sh
#
# Installs the newest bravebot release for this platform.
#
#   curl -fsSL https://raw.githubusercontent.com/brave-experiments/brave-bot/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/brave-experiments/brave-bot/main/install.sh | INSTALL_DIR="$HOME/.local/bin" sh
#
# The checksum check is not optional: without it a network-fetched executable would run on the
# strength of TLS alone, and a substituted release asset would be indistinguishable from a good
# one. Running this again is also how an install made this way is updated.

set -eu

REPO="brave-experiments/brave-bot"
API_URL="https://api.github.com/repos/${REPO}/releases/latest"
BIN_NAME="bravebot"
DEFAULT_INSTALL_DIR="/usr/local/bin"

fail() {
  echo "error: $1" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

# The digest and nothing else: sixty-four hex digits, no filename beside them. A checksum file of
# any other shape is a release published wrong, and installing anyway would make the one signal
# that distinguishes a bad download from a good one meaningless.
is_sha256() {
  case "${#1}" in
    64) ;;
    *) return 1 ;;
  esac
  case "$1" in
    *[!0-9a-fA-F]*) return 1 ;;
  esac
  return 0
}

need_cmd curl
need_cmd mktemp
need_cmd chmod
need_cmd mv
need_cmd rm
need_cmd uname

case "$(uname -s)" in
  Darwin) OS_KEY="darwin" ;;
  Linux) OS_KEY="linux" ;;
  *) fail "unsupported OS: $(uname -s). Windows installs with npm: npm install -g @brave/bravebot" ;;
esac

case "$(uname -m)" in
  arm64 | aarch64) ARCH_KEY="arm64" ;;
  x86_64 | amd64) ARCH_KEY="amd64" ;;
  *) fail "unsupported architecture: $(uname -m)" ;;
esac

# Under Rosetta a translated shell reports x86_64 on an arm64 machine. The x86_64 binary would work
# and would run translated, so take the native one.
if [ "$OS_KEY" = "darwin" ] && [ "$ARCH_KEY" = "amd64" ]; then
  translated="$(sysctl -in sysctl.proc_translated 2>/dev/null || true)"
  native_arm="$(sysctl -in hw.optional.arm64 2>/dev/null || true)"
  if [ "$translated" = "1" ] && [ "$native_arm" = "1" ]; then
    echo "Detected Rosetta translation; installing the native arm64 binary."
    ARCH_KEY="arm64"
  fi
fi

ASSET_NAME="${BIN_NAME}-${OS_KEY}-${ARCH_KEY}"

INSTALL_DIR="${INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"

TAG="$(curl -fsSL "$API_URL" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
[ -n "$TAG" ] || fail "unable to resolve the latest release tag from $API_URL"

BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

BIN_PATH="${TMP_DIR}/${ASSET_NAME}"
SHA_PATH="${TMP_DIR}/${ASSET_NAME}.sha256"

echo "Downloading ${ASSET_NAME} ${TAG}..."
curl -fsSL "${BASE_URL}/${ASSET_NAME}" -o "$BIN_PATH"
curl -fsSL "${BASE_URL}/${ASSET_NAME}.sha256" -o "$SHA_PATH"

EXPECTED="$(tr -d '[:space:]' < "$SHA_PATH")"
is_sha256 "$EXPECTED" || fail "malformed checksum for ${ASSET_NAME}"

if command -v shasum >/dev/null 2>&1; then
  ACTUAL="$(shasum -a 256 "$BIN_PATH" | cut -d ' ' -f 1)"
elif command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "$BIN_PATH" | cut -d ' ' -f 1)"
else
  fail "need shasum or sha256sum to verify the download"
fi

# Lowercased both sides, since the comparison is of two digests and not of two spellings.
EXPECTED="$(printf '%s' "$EXPECTED" | tr 'A-F' 'a-f')"
ACTUAL="$(printf '%s' "$ACTUAL" | tr 'A-F' 'a-f')"
if [ "$ACTUAL" != "$EXPECTED" ]; then
  echo "error: checksum mismatch for ${ASSET_NAME}" >&2
  echo "expected: $EXPECTED" >&2
  echo "actual:   $ACTUAL" >&2
  exit 1
fi

chmod +x "$BIN_PATH"

DEST_PATH="${INSTALL_DIR}/${BIN_NAME}"
if [ ! -d "$INSTALL_DIR" ]; then
  mkdir -p "$INSTALL_DIR" 2>/dev/null || {
    need_cmd sudo
    sudo mkdir -p "$INSTALL_DIR"
  }
fi
if [ -w "$INSTALL_DIR" ]; then
  mv "$BIN_PATH" "$DEST_PATH"
else
  need_cmd sudo
  sudo mv "$BIN_PATH" "$DEST_PATH"
fi

echo "Installed ${BIN_NAME} ${TAG} to ${DEST_PATH}"

ON_PATH="$(command -v "$BIN_NAME" 2>/dev/null || true)"
if [ -z "$ON_PATH" ]; then
  echo "note: ${INSTALL_DIR} is not on your PATH; add it to run ${BIN_NAME} by name."
elif [ "$ON_PATH" != "$DEST_PATH" ]; then
  echo "note: ${ON_PATH} comes first on your PATH, so that is what \`${BIN_NAME}\` still runs."
else
  echo "Run: ${BIN_NAME} --help"
fi
