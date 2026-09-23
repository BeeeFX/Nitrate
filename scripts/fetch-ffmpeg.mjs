// Downloads the ffmpeg/ffprobe sidecars Tauri bundles into the app.
//
// Tauri resolves `externalBin` entries by appending the Rust target triple, so
// the binaries have to land as e.g. `ffmpeg-x86_64-pc-windows-msvc.exe`.
//
// Run automatically via `npm run postinstall`, or by hand with `npm run ffmpeg`.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { chmod, copyFile, mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "src-tauri", "binaries");

// One particular build, checked against its checksum.
//
// This used to take BtbN's `latest` tag, which is rebuilt every day under the
// same name: a rebuild months from now wasn't the app that shipped, and there
// was nothing to check the download against, in a job that goes on to sign
// the installer. Month-end builds are kept for years, where the dailies
// between them go after a few weeks, so the pin is always one of those.
//
// To move it: pick the newest `autobuild-YYYY-MM-(28..31)-*` tag from
// https://github.com/BtbN/FFmpeg-Builds/tags, read the release's own
// `checksums.sha256`, and copy the n8.1 win64-gpl and linux64-gpl lines here.
const BTBN_BUILD = "autobuild-2026-08-31-13-27";
const FFMPEG_RELEASE = "n8.1.2-50-g1a748fe2cd";
const BTBN = `https://github.com/BtbN/FFmpeg-Builds/releases/download/${BTBN_BUILD}`;

const SOURCES = {
  "x86_64-pc-windows-msvc": {
    url: `${BTBN}/ffmpeg-${FFMPEG_RELEASE}-win64-gpl-8.1.zip`,
    sha256: "273abb45f3f9f76c303e35ff39f5bb6c23c163ae65f6244a32b7d4a7f6cf0616",
    archive: "ffmpeg.zip",
    exe: ".exe",
  },
  "x86_64-unknown-linux-gnu": {
    url: `${BTBN}/ffmpeg-${FFMPEG_RELEASE}-linux64-gpl-8.1.tar.xz`,
    sha256: "c733b4b2951e5957e15505f788b2c65a7a41b6da4b289e295852cc38079b4d2b",
    archive: "ffmpeg.tar.xz",
    exe: "",
  },
  // evermeet.cx ships ffmpeg and ffprobe as separate archives, and x86_64 only
  // — which runs fine on Apple Silicon under Rosetta 2.
  //
  // Not pinned: evermeet only serves its newest release at these addresses,
  // so there's no fixed file to check. Fine while macOS isn't released (see
  // release.yml), and the first thing to fix when it is.
  "x86_64-apple-darwin": {
    split: {
      ffmpeg: "https://evermeet.cx/ffmpeg/getrelease/ffmpeg/zip",
      ffprobe: "https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip",
    },
    exe: "",
  },
  "aarch64-apple-darwin": {
    split: {
      ffmpeg: "https://evermeet.cx/ffmpeg/getrelease/ffmpeg/zip",
      ffprobe: "https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip",
    },
    exe: "",
  },
};

function hostTriple() {
  if (process.env.NITRATE_TARGET) return process.env.NITRATE_TARGET;
  const arch = process.arch === "arm64" ? "aarch64" : "x86_64";
  switch (process.platform) {
    case "win32":
      return `${arch}-pc-windows-msvc`;
    case "darwin":
      return `${arch}-apple-darwin`;
    case "linux":
      return `${arch}-unknown-linux-gnu`;
    default:
      throw new Error(`Unsupported platform: ${process.platform}`);
  }
}

async function download(url, dest, sha256) {
  process.stdout.write(`  fetching ${url}\n`);
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`${res.status} ${res.statusText} for ${url}`);
  const bytes = Buffer.from(await res.arrayBuffer());

  const actual = createHash("sha256").update(bytes).digest("hex");
  if (sha256 && actual !== sha256) {
    throw new Error(`checksum mismatch for ${url}\n  expected ${sha256}\n  got      ${actual}`);
  }
  if (!sha256) process.stdout.write(`  !  not pinned, so not verified (sha256 ${actual})\n`);

  await writeFile(dest, bytes);
}

// bsdtar ships with Windows 10+, macOS and most Linux distros, and auto-detects
// zip as well as tar.xz — which saves pulling in an extraction dependency.
function extract(archive, cwd) {
  execFileSync("tar", ["-xf", archive], { cwd, stdio: "inherit" });
}

/** Walks an extracted tree looking for a binary by name. */
async function findBinary(dir, name) {
  const entries = await readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      const hit = await findBinary(full, name);
      if (hit) return hit;
    } else if (entry.name === name) {
      return full;
    }
  }
  return null;
}

async function main() {
  const triple = hostTriple();
  const source = SOURCES[triple];
  if (!source) throw new Error(`No ffmpeg source configured for ${triple}`);

  const ext = source.exe;
  await mkdir(outDir, { recursive: true });

  const targets = ["ffmpeg", "ffprobe"].map((name) => ({
    name,
    file: `${name}${ext}`,
    dest: join(outDir, `${name}-${triple}${ext}`),
  }));

  if (!process.env.NITRATE_FORCE_FFMPEG && targets.every((t) => existsSync(t.dest))) {
    console.log(`ffmpeg sidecars already present for ${triple} — skipping.`);
    return;
  }

  const work = await mkdtemp(join(tmpdir(), "nitrate-ffmpeg-"));
  console.log(`Fetching ffmpeg ${FFMPEG_RELEASE} for ${triple}`);

  try {
    if (source.split) {
      // macOS: one archive per binary.
      for (const target of targets) {
        const archive = join(work, `${target.name}.zip`);
        await download(source.split[target.name], archive, null);
        const sub = join(work, target.name);
        await mkdir(sub, { recursive: true });
        extract(archive, sub);
        const found = await findBinary(sub, target.file);
        if (!found) throw new Error(`${target.file} missing from archive`);
        await copyFile(found, target.dest);
      }
    } else {
      const archive = join(work, source.archive);
      await download(source.url, archive, source.sha256);
      extract(archive, work);
      for (const target of targets) {
        const found = await findBinary(work, target.file);
        if (!found) throw new Error(`${target.file} missing from archive`);
        await copyFile(found, target.dest);
      }
    }

    if (process.platform !== "win32") {
      for (const target of targets) await chmod(target.dest, 0o755);
    }

    for (const target of targets) console.log(`  -> ${target.dest}`);
    console.log("Done.");
  } finally {
    await rm(work, { recursive: true, force: true });
  }
}

main().catch((err) => {
  console.error(`\nCouldn't fetch ffmpeg: ${err.message}`);
  console.error("The app needs ffmpeg/ffprobe in src-tauri/binaries to build.");
  process.exit(1);
});
