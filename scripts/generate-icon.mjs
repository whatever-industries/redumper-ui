import fs from "node:fs/promises";
import path from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";

const execFileAsync = promisify(execFile);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sourceFile = path.join(root, "assets", "source", "icon.png");
const outDir = path.join(root, "src-tauri", "icons");
const pngFile = path.join(outDir, "icon.png");
const icoFile = path.join(outDir, "icon.ico");

await fs.access(sourceFile);
await fs.mkdir(outDir, { recursive: true });

await execFileAsync("python3", [
  "-c",
  `
from PIL import Image
import sys

source, png_output, ico_output = sys.argv[1], sys.argv[2], sys.argv[3]
image = Image.open(source).convert("RGBA")
image = image.resize((512, 512), Image.Resampling.LANCZOS)
image.save(png_output, "PNG")
image.save(ico_output, "ICO", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
`,
  sourceFile,
  pngFile,
  icoFile
]);

console.log(`Generated ${pngFile} and ${icoFile} from ${sourceFile}`);
