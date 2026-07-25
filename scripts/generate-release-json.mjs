import { readFile, writeFile, mkdir, copyFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, "..");
const tauriConfig = JSON.parse(await readFile(path.join(projectDir, "src-tauri", "tauri.conf.json"), "utf8"));
const releaseConfig = JSON.parse(await readFile(path.join(projectDir, "release.config.json"), "utf8"));
const repository = process.env.GITHUB_REPOSITORY || releaseConfig.githubRepository;
const version = tauriConfig.version;
const bundleDir = path.join(projectDir, "src-tauri", "target", "aarch64-apple-darwin", "release", "bundle");
const updaterSource = path.join(bundleDir, "macos", "yt-dlp BD.app.tar.gz");
const signatureSource = `${updaterSource}.sig`;
const dmgSource = path.join(bundleDir, "dmg", `yt-dlp BD_${version}_aarch64.dmg`);
const releaseDir = path.join(projectDir, "release");
const updaterName = `yt-dlp-BD_${version}_aarch64.app.tar.gz`;
const dmgName = `yt-dlp-BD_${version}_aarch64.dmg`;

await mkdir(releaseDir, { recursive: true });
await copyFile(updaterSource, path.join(releaseDir, updaterName));
await copyFile(signatureSource, path.join(releaseDir, `${updaterName}.sig`));
await copyFile(dmgSource, path.join(releaseDir, dmgName));

const signature = (await readFile(signatureSource, "utf8")).trim();
const latest = {
  version,
  notes: process.env.RELEASE_NOTES || "Оновлення yt-dlp BD",
  pub_date: new Date().toISOString(),
  platforms: {
    "darwin-aarch64": {
      signature,
      url: `https://github.com/${repository}/releases/download/v${version}/${updaterName}`,
    },
  },
};

await writeFile(path.join(releaseDir, "latest.json"), `${JSON.stringify(latest, null, 2)}\n`);

if (repository === "OWNER/REPOSITORY") {
  console.warn("Увага: вкажіть свій GitHub owner/repository у release.config.json перед публікацією.");
}
console.log(`Релізні файли підготовлено: ${releaseDir}`);
