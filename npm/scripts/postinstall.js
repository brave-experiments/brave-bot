#!/usr/bin/env node

// Downloads the release binary for this platform and verifies its checksum.
//
// The checksum check is not optional: without it, a network-fetched executable would
// run on the strength of TLS alone, and a compromised or substituted release asset
// would be indistinguishable from a good one.

const fs = require("node:fs");
const path = require("node:path");
const https = require("node:https");
const crypto = require("node:crypto");
const { spawnSync } = require("node:child_process");

const SKIP_ENV = "BUA_INSTALL_SKIP_DOWNLOAD";
const DEFAULT_REPO = "brave-experiments/brave-user-agent";
const MAX_REDIRECTS = 5;

const repo = process.env.BUA_REPO || DEFAULT_REPO;
const pkg = require(path.join(__dirname, "../../package.json"));
const tag = `v${pkg.version}`;

// Lets the package install in CI or a sandbox with no network, and during local
// development where the binary is built rather than downloaded.
if (process.env[SKIP_ENV] === "1") {
  console.log(`Skipping bua binary download because ${SKIP_ENV}=1`);
  process.exit(0);
}

const target = resolveTarget(process.platform, process.arch);
if (!target) {
  console.error(`Unsupported platform/arch: ${process.platform}/${process.arch}`);
  process.exit(1);
}

const baseUrl = `https://github.com/${repo}/releases/download/${tag}`;
const destination = path.join(__dirname, "..", "bin", target.binaryName);

install().catch((error) => {
  console.error(`Failed to install the bua binary: ${error.message}`);
  process.exit(1);
});

async function install() {
  const expected = (await fetchToString(`${baseUrl}/${target.asset}.sha256`)).trim();
  if (!/^[0-9a-f]{64}$/i.test(expected)) {
    throw new Error(`Malformed checksum for ${target.asset}`);
  }

  const bytes = await fetchToBuffer(`${baseUrl}/${target.asset}`);
  const actual = crypto.createHash("sha256").update(bytes).digest("hex");
  if (actual !== expected.toLowerCase()) {
    throw new Error(
      `Checksum mismatch for ${target.asset}: expected ${expected}, got ${actual}`
    );
  }

  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.writeFileSync(destination, bytes, { mode: 0o755 });
  console.log(`Installed bua ${tag} (${target.asset})`);
}

function resolveTarget(platform, arch) {
  const resolved = resolveArch(platform, arch);
  const key = `${platform}-${resolved}`;
  const table = {
    "darwin-arm64": "bua-darwin-arm64",
    "darwin-x64": "bua-darwin-amd64",
    "linux-arm64": "bua-linux-arm64",
    "linux-x64": "bua-linux-amd64",
    "win32-arm64": "bua-windows-arm64.exe",
    "win32-x64": "bua-windows-amd64.exe",
  };
  const asset = table[key];
  if (!asset) {
    return null;
  }
  return {
    asset,
    binaryName: platform === "win32" ? "bua-bin.exe" : "bua-bin",
  };
}

// Under Rosetta, node reports x64 on an arm64 machine. Installing the x64 binary
// would work but run translated, so prefer the native one.
function resolveArch(platform, arch) {
  const override = process.env.BUA_INSTALL_ARCH;
  if (override === "arm64" || override === "x64") {
    return override;
  }

  if (platform === "darwin" && arch === "x64") {
    const translated = sysctl("sysctl.proc_translated");
    const arm64Capable = sysctl("hw.optional.arm64");
    if (translated === "1" && arm64Capable === "1") {
      console.log("Detected Rosetta translation; installing the native arm64 binary.");
      return "arm64";
    }
  }

  return arch;
}

function sysctl(name) {
  const result = spawnSync("sysctl", ["-in", name], { encoding: "utf8" });
  return result.status === 0 ? (result.stdout || "").trim() : null;
}

function fetchToBuffer(url) {
  return new Promise((resolve, reject) => {
    get(url, 0, (error, response) => {
      if (error) {
        reject(error);
        return;
      }
      const chunks = [];
      response.on("data", (chunk) => chunks.push(chunk));
      response.on("end", () => resolve(Buffer.concat(chunks)));
      response.on("error", reject);
    });
  });
}

async function fetchToString(url) {
  return (await fetchToBuffer(url)).toString("utf8");
}

function get(url, redirects, callback) {
  if (redirects > MAX_REDIRECTS) {
    callback(new Error("Too many redirects"));
    return;
  }

  https
    .get(url, { headers: { "User-Agent": "brave-user-agent-installer" } }, (response) => {
      const { statusCode, headers } = response;
      if (statusCode >= 300 && statusCode < 400 && headers.location) {
        response.resume();
        get(new URL(headers.location, url).toString(), redirects + 1, callback);
        return;
      }
      if (statusCode !== 200) {
        response.resume();
        callback(new Error(`HTTP ${statusCode} for ${url}`));
        return;
      }
      callback(null, response);
    })
    .on("error", callback);
}
