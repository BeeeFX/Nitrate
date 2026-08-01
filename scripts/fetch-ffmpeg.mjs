// Downloads the ffmpeg/ffprobe sidecars Tauri bundles into the app.
//
// Tauri resolves `externalBin` entries by appending the Rust target triple, so
// the binaries have to land as e.g. `ffmpeg-x86_64-pc-windows-msvc.exe`.
//
// Run automatically via `npm run postinstall`, or by hand with `npm run ffmpeg`.

import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, copyFile, mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "src-tauri", "binaries");

// Pinned to a release line rather than `master-latest`, so a rebuild months
// from now produces the same app.
const FFMPEG_RELEASE = "n7.1";

const SOURCES = {
  "x86_64-pc-windows-msvc": {
    url: `https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-${FFMPEG_RELEASE}-latest-win64-gpl-7.1.zip`,
    archive: "ffmpeg.zip",
    exe: ".exe",
  },
  "x86_64-unknown-linux-gnu": {
    url: `https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-${FFMPEG_RELEASE}-latest-linux64-gpl-7.1.tar.xz`,
    archive: "ffmpeg.tar.xz",
    exe: "",
  },
  // evermeet.cx ships ffmpeg and ffprobe as separate archives, and x86_64 only
  // — which runs fine on Apple Silicon under Rosetta 2.
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

async function download(url, dest) {
  process.stdout.write(`  fetching ${url}\n`);
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`${res.status} ${res.statusText} for ${url}`);
  await writeFile(dest, Buffer.from(await res.arrayBuffer()));
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
        await download(source.split[target.name], archive);
        const sub = join(work, target.name);
        await mkdir(sub, { recursive: true });
        extract(archive, sub);
        const found = await findBinary(sub, target.file);
        if (!found) throw new Error(`${target.file} missing from archive`);
        await copyFile(found, target.dest);
      }
    } else {
      const archive = join(work, source.archive);
      await download(source.url, archive);
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
