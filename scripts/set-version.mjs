// Writes a single version into the three files that must agree on it:
// package.json, src-tauri/Cargo.toml and src-tauri/tauri.conf.json.
//
// tauri.conf.json's version names every bundle artifact
// (Donut_<version>_aarch64.dmg, …), so a release tag that disagrees with it
// produces assets whose filenames don't match the tag. CI calls this with the
// tag it is building so the bump doesn't have to be a manual commit.
//
// Usage: node scripts/set-version.mjs v0.29.0   (leading "v" optional)

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const raw = process.argv[2];
if (!raw) {
  console.error("Usage: node scripts/set-version.mjs <version>");
  process.exit(1);
}

// Accept "v0.29.0", "0.29.0" and pre-release suffixes like "v0.29.0-fork.1".
const version = raw.replace(/^v/, "");
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(
    `Invalid version "${raw}" — expected semver such as 0.29.0 or v0.29.0-fork.1`,
  );
  process.exit(1);
}

// RPM's Version field forbids "-", and Tauri feeds this string to the bundler
// verbatim, so a pre-release tag breaks the rpm target specifically.
if (version.includes("-")) {
  console.warn(
    `Warning: "${version}" has a pre-release suffix; the rpm bundle target will reject it. Use a plain X.Y.Z tag for full-matrix releases.`,
  );
}

function replaceInFile(relPath, pattern, replacement) {
  const path = join(ROOT, relPath);
  const before = readFileSync(path, "utf-8");
  const after = before.replace(pattern, replacement);
  if (before === after) {
    console.error(
      `Failed to set version in ${relPath} (pattern did not match)`,
    );
    process.exit(1);
  }
  writeFileSync(path, after);
  console.log(`${relPath} -> ${version}`);
}

// Only the first `version = "..."` is the [package] one; workspace and
// dependency entries come later.
replaceInFile(
  "src-tauri/Cargo.toml",
  /^version = "[^"]*"$/m,
  `version = "${version}"`,
);
replaceInFile(
  "src-tauri/tauri.conf.json",
  /"version": "[^"]*"/,
  `"version": "${version}"`,
);
replaceInFile("package.json", /"version": "[^"]*"/, `"version": "${version}"`);
