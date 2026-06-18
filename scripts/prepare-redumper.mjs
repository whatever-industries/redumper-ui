import crypto from "node:crypto";
import fs from "node:fs/promises";
import { createWriteStream } from "node:fs";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import extract from "extract-zip";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = path.join(root, ".redumper", "upstream.json");
const resourceDir = path.join(root, "src-tauri", "resources", "redumper");
const downloadDir = path.join(root, ".redumper", "downloads");

const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
const target = process.env.REDUMPER_TARGET || detectTarget();
const asset = manifest.assets[target];
const assetTag = asset?.tag || manifest.tag;

if (!asset) {
  throw new Error(`No redumper asset pinned for target "${target}"`);
}

await fs.mkdir(downloadDir, { recursive: true });
await fs.mkdir(resourceDir, { recursive: true });

const zipPath = path.join(downloadDir, asset.name);
await download(asset.url, zipPath);

const actual = await sha256(zipPath);
if (actual !== asset.sha256) {
  throw new Error(`SHA-256 mismatch for ${asset.name}: expected ${asset.sha256}, got ${actual}`);
}

for (const entry of await fs.readdir(resourceDir)) {
  if (entry !== ".gitkeep") {
    await fs.rm(path.join(resourceDir, entry), { recursive: true, force: true });
  }
}

await extract(zipPath, { dir: resourceDir });
await flattenSingleArchiveRoot(resourceDir);

if (!target.startsWith("windows-")) {
  const executable = path.join(resourceDir, "bin", "redumper");
  await fs.chmod(executable, 0o755).catch(() => undefined);
}

console.log(`Prepared redumper ${assetTag} for ${target}`);

async function flattenSingleArchiveRoot(directory) {
  const entries = (await fs.readdir(directory, { withFileTypes: true })).filter((entry) => entry.name !== ".gitkeep");
  if (entries.length !== 1 || !entries[0].isDirectory()) {
    return;
  }

  const rootDir = path.join(directory, entries[0].name);
  for (const entry of await fs.readdir(rootDir)) {
    await fs.rename(path.join(rootDir, entry), path.join(directory, entry));
  }
  await fs.rmdir(rootDir);
}

function detectTarget() {
  const platform = {
    darwin: "macos",
    linux: "linux",
    win32: "windows"
  }[process.platform];

  const arch = {
    arm64: "arm64",
    x64: "x64"
  }[process.arch];

  if (!platform || !arch) {
    throw new Error(`Unsupported host target ${process.platform}/${process.arch}`);
  }

  return `${platform}-${arch}`;
}

async function download(url, destination) {
  await new Promise((resolve, reject) => {
    const request = https.get(url, (response) => {
      if ([301, 302, 303, 307, 308].includes(response.statusCode ?? 0)) {
        response.resume();
        download(response.headers.location, destination).then(resolve, reject);
        return;
      }

      if (response.statusCode !== 200) {
        response.resume();
        reject(new Error(`Download failed with HTTP ${response.statusCode}`));
        return;
      }

      const file = createWriteStream(destination);
      response.pipe(file);
      file.on("finish", () => file.close(resolve));
      file.on("error", reject);
    });
    request.on("error", reject);
  });
}

async function sha256(filePath) {
  const hash = crypto.createHash("sha256");
  const data = await fs.readFile(filePath);
  hash.update(data);
  return hash.digest("hex");
}
