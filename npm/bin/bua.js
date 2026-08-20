#!/usr/bin/env node

// Thin launcher: forwards to the platform binary fetched during install.

const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const { spawnSync } = require("node:child_process");

const binaryName = process.platform === "win32" ? "bua-bin.exe" : "bua-bin";
const binaryPath = path.join(__dirname, binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error(
    "The bua binary is missing. Reinstall the package to download the binary for this platform."
  );
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  console.error(`Failed to run bua: ${result.error.message}`);
  process.exit(1);
}

if (result.status !== null) {
  process.exit(result.status);
}

// Killed by a signal: report it the way a shell would, so `bua` behaves like the
// binary it wraps rather than collapsing every signal into a generic failure.
if (result.signal) {
  const signalNumber = os.constants.signals[result.signal];
  process.exit(typeof signalNumber === "number" ? 128 + signalNumber : 1);
}

process.exit(1);
