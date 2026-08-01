// Bumps the version in the three places that have to agree, then tags.
//
// The updater compares the running app's version against latest.json, so a
// release where these drift either never offers the update or offers it forever.
//
//   node scripts/bump-version.mjs 0.2.0

import { execFileSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const version = process.argv[2];

if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error("Usage: node scripts/bump-version.mjs <major.minor.patch>");
  process.exit(1);
}

async function patch(file, transform) {
  const path = join(root, file);
  const before = await readFile(path, "utf8");
  const after = transform(before);
  if (before === after) {
    console.error(`  !  no version field matched in ${file}`);
    process.exit(1);
  }
  await writeFile(path, after);
  console.log(`  ${file}`);
}

console.log(`Bumping to ${version}:`);

// Only the first "version" key — that's the package's own.
await patch("package.json", (s) =>
  s.replace(/"version":\s*"[^"]+"/, `"version": "${version}"`),
);

await patch("src-tauri/tauri.conf.json", (s) =>
  s.replace(/"version":\s*"[^"]+"/, `"version": "${version}"`),
);

// Anchored to [package] so a dependency's version can't be hit by accident.
await patch("src-tauri/Cargo.toml", (s) =>
  s.replace(/(\[package\][\s\S]*?\nversion = ")[^"]+(")/, `$1${version}$2`),
);

// Keeps Cargo.lock in step so CI doesn't fail on a dirty lockfile.
try {
  execFileSync("cargo", ["update", "--workspace", "--offline"], {
    cwd: join(root, "src-tauri"),
    stdio: "ignore",
  });
  console.log("  src-tauri/Cargo.lock");
} catch {
  console.log("  !  couldn't refresh Cargo.lock — run `cargo check` before committing");
}

console.log(`
Next:
  git commit -am "Release v${version}"
  git tag v${version}
  git push && git push --tags

CI builds the installers and opens a draft release.
Publish it — auto-update only sees published releases.`);
