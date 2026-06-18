import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = path.join(root, ".redumper", "upstream.json");
const packagePath = path.join(root, "package.json");
const packageLockPath = path.join(root, "package-lock.json");
const cargoPath = path.join(root, "src-tauri", "Cargo.toml");
const tauriConfigPath = path.join(root, "src-tauri", "tauri.conf.json");

const targets = ["linux-arm64", "linux-x64", "macos-arm64", "macos-x64", "windows-arm64", "windows-x64"];

const release = await fetchJson("https://api.github.com/repos/superg/redumper/releases/latest");
const tag = release.tag_name;
const appVersion = appVersionFromTag(tag);

const assets = {};
for (const target of targets) {
  const assetName = `redumper-${tag}-${target}.zip`;
  const asset = release.assets.find((candidate) => candidate.name === assetName);
  if (!asset) {
    throw new Error(`Missing upstream release asset: ${assetName}`);
  }
  assets[target] = {
    name: asset.name,
    url: asset.browser_download_url,
    sha256: String(asset.digest || "").replace(/^sha256:/, "")
  };
  if (!assets[target].sha256) {
    throw new Error(`Missing SHA-256 digest for ${asset.name}`);
  }
}

const manifest = {
  owner: "superg",
  repo: "redumper",
  tag,
  publishedAt: release.published_at,
  appVersion,
  assets
};

await fs.writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

const packageJson = JSON.parse(await fs.readFile(packagePath, "utf8"));
packageJson.version = appVersion;
await fs.writeFile(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`);

if (await exists(packageLockPath)) {
  const packageLock = JSON.parse(await fs.readFile(packageLockPath, "utf8"));
  packageLock.version = appVersion;
  if (packageLock.packages?.[""]) {
    packageLock.packages[""].version = appVersion;
  }
  await fs.writeFile(packageLockPath, `${JSON.stringify(packageLock, null, 2)}\n`);
}

let cargoToml = await fs.readFile(cargoPath, "utf8");
cargoToml = cargoToml.replace(/^version = ".*"$/m, `version = "${appVersion}"`);
await fs.writeFile(cargoPath, cargoToml);

const tauriConfig = JSON.parse(await fs.readFile(tauriConfigPath, "utf8"));
tauriConfig.version = appVersion;
await fs.writeFile(tauriConfigPath, `${JSON.stringify(tauriConfig, null, 2)}\n`);

console.log(`Pinned redumper ${tag} and app version ${appVersion}`);

async function fetchJson(url) {
  const response = await fetch(url, {
    headers: {
      Accept: "application/vnd.github+json",
      "User-Agent": "redumper-ui-updater"
    }
  });
  if (!response.ok) {
    throw new Error(`GitHub API request failed: ${response.status} ${response.statusText}`);
  }
  return response.json();
}

function appVersionFromTag(tag) {
  const match = /^b(\d+)$/.exec(tag);
  if (!match) {
    throw new Error(`Cannot derive SemVer from upstream tag "${tag}"`);
  }
  return `0.1.${Number(match[1])}`;
}

async function exists(filePath) {
  try {
    await fs.access(filePath);
    return true;
  } catch {
    return false;
  }
}
