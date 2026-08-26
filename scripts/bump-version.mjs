#!/usr/bin/env node
// Sync the app version across package.json, src-tauri/Cargo.toml ([package]
// version), src-tauri/tauri.conf.json and this crate's entry in
// src-tauri/Cargo.lock. Zero dependencies (plain Node).
//
// Usage:
//   node scripts/bump-version.mjs <x.y.z>     explicit version
//   node scripts/bump-version.mjs patch|minor|major
//
// Refuses to run if the four files currently disagree. Does no git actions.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

const paths = {
  packageJson: join(repoRoot, "package.json"),
  cargoToml: join(repoRoot, "src-tauri", "Cargo.toml"),
  tauriConf: join(repoRoot, "src-tauri", "tauri.conf.json"),
  cargoLock: join(repoRoot, "src-tauri", "Cargo.lock"),
};

const SEMVER = /^(\d+)\.(\d+)\.(\d+)$/;

function fail(message) {
  console.error(`bump-version: ${message}`);
  process.exit(1);
}

// --- read current versions -------------------------------------------------

function readJsonVersion(path) {
  const raw = readFileSync(path, "utf8");
  const data = JSON.parse(raw);
  if (typeof data.version !== "string") {
    fail(`no string "version" field in ${path}`);
  }
  return data.version;
}

// The [package] version is the first `version = "..."` inside the [package]
// table. Anchoring to that table avoids touching dependency versions.
const CARGO_PACKAGE_VERSION =
  /(\[package\][^\[]*?\n\s*version\s*=\s*")([^"]+)(")/;

function readCargoVersion(path) {
  const raw = readFileSync(path, "utf8");
  const match = raw.match(CARGO_PACKAGE_VERSION);
  if (!match) fail(`could not find [package] version in ${path}`);
  return match[2];
}

const CARGO_PACKAGE_NAME = /\[package\][^\[]*?\n\s*name\s*=\s*"([^"]+)"/;

function readCargoName(path) {
  const raw = readFileSync(path, "utf8");
  const match = raw.match(CARGO_PACKAGE_NAME);
  if (!match) fail(`could not find [package] name in ${path}`);
  return match[1];
}

// This crate's own entry in the lockfile, anchored on its `name` line so no
// dependency's version can be caught by the replace.
const crateName = readCargoName(paths.cargoToml);
const CARGO_LOCK_VERSION = new RegExp(
  `(name\\s*=\\s*"${crateName}"\\s*\\n\\s*version\\s*=\\s*")([^"]+)(")`,
);

function readLockVersion(path) {
  const raw = readFileSync(path, "utf8");
  const match = raw.match(CARGO_LOCK_VERSION);
  if (!match) fail(`could not find the "${crateName}" package in ${path}`);
  return match[2];
}

const current = {
  packageJson: readJsonVersion(paths.packageJson),
  cargoToml: readCargoVersion(paths.cargoToml),
  tauriConf: readJsonVersion(paths.tauriConf),
  cargoLock: readLockVersion(paths.cargoLock),
};

// --- validate agreement ----------------------------------------------------

const distinct = [...new Set(Object.values(current))];
if (distinct.length !== 1) {
  const detail = Object.entries(current)
    .map(([file, version]) => `  ${file}: ${version}`)
    .join("\n");
  fail(
    `versions disagree; fix them to match before bumping:\n${detail}`,
  );
}
const currentVersion = distinct[0];
if (!SEMVER.test(currentVersion)) {
  fail(`current version "${currentVersion}" is not x.y.z`);
}

// --- compute the next version ----------------------------------------------

const arg = process.argv[2];
if (!arg) {
  fail("missing argument: <x.y.z> or patch|minor|major");
}

function nextVersion(from, spec) {
  if (SEMVER.test(spec)) return spec;
  const [major, minor, patch] = from.match(SEMVER).slice(1).map(Number);
  switch (spec) {
    case "major":
      return `${major + 1}.0.0`;
    case "minor":
      return `${major}.${minor + 1}.0`;
    case "patch":
      return `${major}.${minor}.${patch + 1}`;
    default:
      fail(`invalid argument "${spec}"; use x.y.z or patch|minor|major`);
  }
}

const newVersion = nextVersion(currentVersion, arg);
if (newVersion === currentVersion) {
  fail(`version is already ${newVersion}; nothing to do`);
}

// --- write the four files --------------------------------------------------

function writeJsonVersion(path, version) {
  const data = JSON.parse(readFileSync(path, "utf8"));
  data.version = version;
  writeFileSync(path, `${JSON.stringify(data, null, 2)}\n`);
}

function writeTomlVersion(path, pattern, version) {
  const raw = readFileSync(path, "utf8");
  const updated = raw.replace(
    pattern,
    (_m, before, _old, after) => `${before}${version}${after}`,
  );
  writeFileSync(path, updated);
}

writeJsonVersion(paths.packageJson, newVersion);
writeTomlVersion(paths.cargoToml, CARGO_PACKAGE_VERSION, newVersion);
writeJsonVersion(paths.tauriConf, newVersion);
writeTomlVersion(paths.cargoLock, CARGO_LOCK_VERSION, newVersion);

console.log(`bump-version: ${currentVersion} -> ${newVersion}`);
console.log(`  package.json`);
console.log(`  src-tauri/Cargo.toml`);
console.log(`  src-tauri/tauri.conf.json`);
console.log(`  src-tauri/Cargo.lock`);
