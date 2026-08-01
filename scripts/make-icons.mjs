// Renders assets/icon.svg to the master PNG that `tauri icon` fans out from.
import sharp from "sharp";
import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const src = resolve(root, "assets/icon.svg");
const out = resolve(root, "assets/icon.png");

await mkdir(dirname(out), { recursive: true });
await sharp(src, { density: 384 }).resize(1024, 1024).png().toFile(out);

console.log(`Wrote ${out}`);
