// Packages the extension for both browsers.
//
// One source tree, two manifests: Chrome wants a service worker, Firefox wants
// background scripts and a stable add-on id. Everything else is shared, so the
// build just copies `src/` and drops the right manifest on top.

import { execFileSync } from "node:child_process";
import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = join(root, "extension");
const outRoot = join(root, "extension", "dist");

const TARGETS = [
  { name: "chrome", manifest: "manifest.chrome.json" },
  { name: "firefox", manifest: "manifest.firefox.json" },
];

/** Keeps the extension's version in step with the app's. */
async function appVersion() {
  const pkg = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
  return pkg.version;
}

async function buildIcons() {
  const iconDir = join(source, "icons");
  await mkdir(iconDir, { recursive: true });
  const svg = join(root, "assets", "icon.svg");

  for (const size of [16, 48, 128]) {
    await sharp(svg, { density: 384 })
      .resize(size, size)
      .png()
      .toFile(join(iconDir, `${size}.png`));
  }
}

async function build() {
  const version = await appVersion();
  await buildIcons();
  await rm(outRoot, { recursive: true, force: true });

  for (const target of TARGETS) {
    const out = join(outRoot, target.name);
    await mkdir(out, { recursive: true });

    await cp(join(source, "src"), join(out, "src"), { recursive: true });
    await cp(join(source, "icons"), join(out, "icons"), { recursive: true });

    const manifest = JSON.parse(
      await readFile(join(source, target.manifest), "utf8"),
    );
    manifest.version = version;
    await writeFile(
      join(out, "manifest.json"),
      `${JSON.stringify(manifest, null, 2)}\n`,
    );

    // bsdtar ships with Windows 10+, macOS and most Linux distributions, so
    // zipping needs no dependency.
    const zip = join(outRoot, `nitrate-extension-${target.name}-${version}.zip`);
    execFileSync("tar", ["-a", "-c", "-f", zip, "-C", out, "."], {
      stdio: "inherit",
    });

    console.log(`  ${target.name}: ${zip}`);
  }
}

build().catch((err) => {
  console.error(err.message);
  process.exit(1);
});
