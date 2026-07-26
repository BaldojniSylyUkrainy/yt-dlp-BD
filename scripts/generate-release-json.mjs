import { copyFile, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, "..");

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  return index === -1 ? null : process.argv[index + 1] || null;
}

export function expectedAssetNames(releaseVersion) {
  return {
    macUpdater: `yt-dlp-BD_${releaseVersion}_aarch64.app.tar.gz`,
    macDmg: `yt-dlp-BD_${releaseVersion}_aarch64.dmg`,
    windowsInstaller: `yt-dlp-BD_${releaseVersion}_windows-x86_64-setup.exe`,
  };
}

export function buildReleaseManifest({
  assetNames,
  signatures,
  repository,
  releaseVersion,
  updaterVersion,
  notes,
  pubDate,
  requireAll = false,
}) {
  const expected = expectedAssetNames(releaseVersion);
  const platforms = {};

  const addUpdater = (fileName, keys) => {
    if (!assetNames.includes(fileName)) return;
    const signature = signatures[fileName];
    if (!signature) {
      throw new Error(`Missing or empty updater signature for ${fileName}`);
    }
    const entry = {
      signature,
      url: `https://github.com/${repository}/releases/download/v${releaseVersion}/${fileName}`,
    };
    for (const key of keys) platforms[key] = entry;
  };

  addUpdater(expected.macUpdater, ["darwin-aarch64"]);
  addUpdater(expected.windowsInstaller, ["windows-x86_64-nsis", "windows-x86_64"]);

  if (requireAll) {
    for (const required of ["darwin-aarch64", "windows-x86_64"]) {
      if (!platforms[required]) {
        throw new Error(`Missing required updater platform: ${required}`);
      }
    }
    if (!assetNames.includes(expected.macDmg)) {
      throw new Error(`Missing required installer: ${expected.macDmg}`);
    }
  }
  if (Object.keys(platforms).length === 0) {
    throw new Error("No recognized updater artifacts found");
  }

  return {
    version: updaterVersion,
    notes,
    pub_date: pubDate,
    platforms,
  };
}

async function prepareMacAssets({ releaseDir, releaseVersion, updaterVersion }) {
  const bundleDir = path.join(
    projectDir,
    "src-tauri",
    "target",
    "aarch64-apple-darwin",
    "release",
    "bundle",
  );
  const updaterSource = path.join(bundleDir, "macos", "yt-dlp BD.app.tar.gz");
  const signatureSource = `${updaterSource}.sig`;
  const dmgSource = path.join(bundleDir, "dmg", `yt-dlp BD_${updaterVersion}_aarch64.dmg`);
  const expected = expectedAssetNames(releaseVersion);

  await mkdir(releaseDir, { recursive: true });
  await copyFile(updaterSource, path.join(releaseDir, expected.macUpdater));
  await copyFile(signatureSource, path.join(releaseDir, `${expected.macUpdater}.sig`));
  await copyFile(dmgSource, path.join(releaseDir, expected.macDmg));
}

async function main() {
  const tauriConfig = JSON.parse(
    await readFile(path.join(projectDir, "src-tauri", "tauri.conf.json"), "utf8"),
  );
  const packageJson = JSON.parse(await readFile(path.join(projectDir, "package.json"), "utf8"));
  const releaseConfig = JSON.parse(
    await readFile(path.join(projectDir, "release.config.json"), "utf8"),
  );
  const repository = process.env.GITHUB_REPOSITORY || releaseConfig.githubRepository;
  const updaterVersion = tauriConfig.version;
  const releaseVersion = packageJson.releaseVersion || updaterVersion;
  const explicitAssetsDir = argumentValue("--assets-dir");
  const releaseDir = explicitAssetsDir
    ? path.resolve(explicitAssetsDir)
    : path.join(projectDir, "release");

  if (!explicitAssetsDir) {
    await prepareMacAssets({ releaseDir, releaseVersion, updaterVersion });
  }

  const assetNames = await readdir(releaseDir);
  const expected = expectedAssetNames(releaseVersion);
  const signatures = {};
  for (const fileName of [expected.macUpdater, expected.windowsInstaller]) {
    if (!assetNames.includes(fileName)) continue;
    const signaturePath = path.join(releaseDir, `${fileName}.sig`);
    signatures[fileName] = (await readFile(signaturePath, "utf8")).trim();
  }
  const manifest = buildReleaseManifest({
    assetNames,
    signatures,
    repository,
    releaseVersion,
    updaterVersion,
    notes: process.env.RELEASE_NOTES || `Оновлення yt-dlp BD ${releaseVersion}`,
    pubDate: new Date().toISOString(),
    requireAll: process.argv.includes("--require-all"),
  });

  await writeFile(
    path.join(releaseDir, "latest.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  console.log(`Релізні файли підготовлено: ${releaseDir}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
