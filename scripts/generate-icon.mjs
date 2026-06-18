import fs from "node:fs/promises";
import path from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";

const execFileAsync = promisify(execFile);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sourceFile = path.join(root, "assets", "icon.jpeg");
const outDir = path.join(root, "src-tauri", "icons");
const outFile = path.join(outDir, "icon.png");

await fs.access(sourceFile);
await fs.mkdir(outDir, { recursive: true });

await execFileAsync("python3", [
  "-c",
  `
from PIL import Image
import sys

source, output = sys.argv[1], sys.argv[2]
image = Image.open(source).convert("RGBA")
image = image.resize((512, 512), Image.Resampling.LANCZOS)
image.save(output, "PNG")
`,
  sourceFile,
  outFile
]);

console.log(`Generated ${outFile} from ${sourceFile}`);
