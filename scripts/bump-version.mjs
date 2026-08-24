#!/usr/bin/env node
// Sync the app version across package.json, src-tauri/Cargo.toml ([package]
// version) and src-tauri/tauri.conf.json. Zero dependencies (plain Node).
//
// Usage:
//   node scripts/bump-version.mjs <x.y.z>     explicit version
//   node scripts/bump-version.mjs patch|minor|major
//
// Refuses to run if the three files currently disagree. Does no git actions.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

const paths = {
  packageJson: join(repoRoot, "package.json"),
  cargoToml: join(repoRoot, "src-tauri", "Cargo.toml"),
  tauriConf: join(repoRoot, "src-tauri", "tauri.conf.json"),
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

const current = {
  packageJson: readJsonVersion(paths.packageJson),
  cargoToml: readCargoVersion(paths.cargoToml),
  tauriConf: readJsonVersion(paths.tauriConf),
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

// --- write the three files -------------------------------------------------

function writeJsonVersion(path, version) {
  const data = JSON.parse(readFileSync(path, "utf8"));
  data.version = version;
  writeFileSync(path, `${JSON.stringify(data, null, 2)}\n`);
}

function writeCargoVersion(path, version) {
  const raw = readFileSync(path, "utf8");
  const updated = raw.replace(
    CARGO_PACKAGE_VERSION,
    (_m, before, _old, after) => `${before}${version}${after}`,
  );
  writeFileSync(path, updated);
}

writeJsonVersion(paths.packageJson, newVersion);
writeCargoVersion(paths.cargoToml, newVersion);
writeJsonVersion(paths.tauriConf, newVersion);

console.log(`bump-version: ${currentVersion} -> ${newVersion}`);
console.log(`  package.json`);
console.log(`  src-tauri/Cargo.toml`);
console.log(`  src-tauri/tauri.conf.json`);
