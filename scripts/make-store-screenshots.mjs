// Turns the README screenshots into Chrome Web Store screenshots.
//
// The store wants exactly 1280x800 (or 640x400), JPEG or 24-bit PNG, and it
// rejects anything carrying an alpha channel without explaining why. Our
// screenshots are none of those things: they're the bare app window, 440x661
// with transparent rounded corners.
//
// Stretching one to fit would distort it, and padding it flat would leave the
// window stranded in a void. So each is centred on a dark backdrop with a soft
// shadow under it, which is what a store listing looks like anyway.

import { createHash } from "node:crypto";
import { mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = join(root, "docs");
const out = join(root, "docs", "store", "screenshots");

const WIDTH = 1280;
const HEIGHT = 800;

/** How much of the canvas height the window fills, leaving it room to breathe. */
const FILL = 0.85;

/**
 * In the order they tell the story; the store shows them like this.
 *
 * Five, because that's Chrome's limit — a sixth would have to be dropped at
 * upload time by whoever is least sure which one matters.
 */
const SHOTS = [
  ["screenshot-empty.png", "01-drop-a-video"],
  ["screenshot-done.png", "02-finished"],
  ["screenshot-editor.png", "03-crop-and-trim"],
  ["screenshot-long.png", "04-long-videos"],
  ["screenshot-settings.png", "05-settings"],
];

/**
 * The backdrop: the app's own near-black, lifted by a blurple glow behind
 * where the window sits, so the shot has some depth without competing with it.
 */
function backdrop() {
  return Buffer.from(`
    <svg width="${WIDTH}" height="${HEIGHT}" xmlns="http://www.w3.org/2000/svg">
      <defs>
        <radialGradient id="glow" cx="50%" cy="38%" r="62%">
          <stop offset="0%" stop-color="#2b3170"/>
          <stop offset="55%" stop-color="#14161f"/>
          <stop offset="100%" stop-color="#0b0c11"/>
        </radialGradient>
      </defs>
      <rect width="${WIDTH}" height="${HEIGHT}" fill="url(#glow)"/>
    </svg>`);
}

/**
 * A shadow shaped like the window itself.
 *
 * Built from the screenshot's own alpha channel, so it follows the rounded
 * corners exactly rather than being a rectangle poking out behind them.
 */
async function shadowFor(image, width, height) {
  const alpha = await sharp(image)
    .extractChannel("alpha")
    .blur(34)
    // Held well below full black: at full strength the offset copy reads as a
    // bar sitting under the window rather than as a shadow cast by it.
    .linear(0.42, 0)
    .toBuffer();

  return sharp({
    create: {
      width,
      height,
      channels: 3,
      background: { r: 0, g: 0, b: 0 },
    },
  })
    .joinChannel(alpha)
    .png()
    .toBuffer();
}

/**
 * Refuses to ship the same picture twice.
 *
 * `screenshot-progress.png` was a byte-for-byte copy of `screenshot-done.png`
 * when this was written — the README had been showing the finished queue under
 * a caption about progress bars. Nothing catches that by eye at thumbnail size,
 * and a store listing with a repeated image looks careless.
 */
async function refuseDuplicates() {
  const seen = new Map();
  for (const [file] of SHOTS) {
    const hash = createHash("md5")
      .update(await readFile(join(source, file)))
      .digest("hex");
    const first = seen.get(hash);
    if (first) {
      throw new Error(
        `${file} is identical to ${first} — one of them is the wrong capture.`,
      );
    }
    seen.set(hash, file);
  }
}

async function build() {
  await refuseDuplicates();
  await rm(out, { recursive: true, force: true });
  await mkdir(out, { recursive: true });

  for (const [file, name] of SHOTS) {
    const scaled = await sharp(join(source, file))
      .resize({ height: Math.round(HEIGHT * FILL), fit: "inside" })
      .png()
      .toBuffer();

    const { width, height } = await sharp(scaled).metadata();
    const shadow = await shadowFor(scaled, width, height);

    const left = Math.round((WIDTH - width) / 2);
    const top = Math.round((HEIGHT - height) / 2);

    await sharp(backdrop())
      .composite([
        // Offset downwards so the light reads as coming from above.
        { input: shadow, left, top: top + 22, blend: "over" },
        { input: scaled, left, top },
      ])
      // Chrome rejects RGBA without saying so, and these need both calls:
      // `flatten` composites the transparency away but leaves the channel in
      // place, so the file still writes as 32-bit until `removeAlpha` drops it.
      .flatten({ background: "#0b0c11" })
      .removeAlpha()
      .png({ compressionLevel: 9 })
      .toFile(join(out, `${name}.png`));
  }

  // Prove it rather than trust it: the two things the store silently rejects
  // are the wrong size and a stray alpha channel.
  const written = (await readdir(out)).filter((f) => f.endsWith(".png")).sort();
  const report = [];
  for (const file of written) {
    const meta = await sharp(join(out, file)).metadata();
    const ok = meta.width === WIDTH && meta.height === HEIGHT && meta.channels === 3;
    report.push(`  ${ok ? "ok  " : "BAD "} ${file}  ${meta.width}x${meta.height}  ${meta.channels} channels`);
    if (!ok) process.exitCode = 1;
  }

  console.log(report.join("\n"));
  console.log(`\n  ${written.length} screenshots in docs/store/screenshots/`);

  await writeFile(
    join(out, "README.md"),
    `# Store screenshots\n\n` +
      `Generated by \`npm run store:shots\` from the screenshots in \`docs/\`.\n` +
      `Don't edit them by hand — regenerate instead.\n\n` +
      `1280x800, 24-bit PNG, no alpha channel — what the Chrome Web Store\n` +
      `accepts. Five of them, which is its limit, numbered in the order they're\n` +
      `worth showing in. Firefox takes the same files and isn't fussy about\n` +
      `size.\n`,
  );
}

build().catch((err) => {
  console.error(err.message);
  process.exit(1);
});
