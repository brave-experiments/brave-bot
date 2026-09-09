#!/usr/bin/env node

// Thin launcher: forwards to the platform binary fetched during install.

const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const { spawnSync } = require("node:child_process");

const binaryName = process.platform === "win32" ? "bravebot-bin.exe" : "bravebot-bin";
const binaryPath = path.join(__dirname, binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error(
    "The bravebot binary is missing. Reinstall the package to download the binary for this platform."
  );
  process.exit(1);
}

// The binary is an ordinary executable in whatever directory npm unpacked it into, so this
// launcher is the only thing that knows the package it belongs to. It says so, and that is what
// lets bravebot offer the npm command when a newer release is out rather than one that would
// install a second copy somewhere else.
const result = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: "inherit",
  env: { ...process.env, BRAVEBOT_INSTALLED_VIA: "npm" },
});

if (result.error) {
  console.error(`Failed to run bravebot: ${result.error.message}`);
  process.exit(1);
}

if (result.status !== null) {
  process.exit(result.status);
}

// Killed by a signal: report it the way a shell would, so `bravebot` behaves like the
// binary it wraps rather than collapsing every signal into a generic failure.
if (result.signal) {
  const signalNumber = os.constants.signals[result.signal];
  process.exit(typeof signalNumber === "number" ? 128 + signalNumber : 1);
}

process.exit(1);
