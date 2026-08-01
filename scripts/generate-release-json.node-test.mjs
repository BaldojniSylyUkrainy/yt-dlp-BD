import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import {
  buildReleaseManifest,
  expectedAssetNames,
  validateReleaseNotes,
} from "./generate-release-json.mjs";

const releaseVersion = "0.2.0.0";
const updaterVersion = "0.2.0";
const expected = expectedAssetNames(releaseVersion);

test("uses stable human-readable names for every public release asset", () => {
  assert.deepEqual(expected, {
    macUpdater: "BaldojnyiDownloader-0.2.0.0-Mac-Apple-Silicon-AutoUpdate.app.tar.gz",
    macDmg: "BaldojnyiDownloader-0.2.0.0-Mac-Apple-Silicon.dmg",
    windowsInstaller: "BaldojnyiDownloader-0.2.0.0-Windows-x64-Setup.exe",
  });
});

test("uses the public product name and version for GitHub Release titles", async () => {
  const workflow = await readFile(".github/workflows/release.yml", "utf8");
  const expectedTitle = '--title "BaldojnyiDownloader $RELEASE_VERSION"';
  assert.equal(workflow.match(new RegExp(expectedTitle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "g"))?.length, 2);
  assert.match(workflow, /description: "Four-part release tag from package\.json, for example vX\.Y\.Z\.H"/);
});

test("uses the branded installed app name and Windows installer icons", async () => {
  const tauriConfig = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
  const macBuild = await readFile("scripts/build-macos.sh", "utf8");
  const releaseGenerator = await readFile("scripts/generate-release-json.mjs", "utf8");
  assert.equal(tauriConfig.productName, "Baldojnyi Downloader");
  assert.equal(tauriConfig.bundle.windows.nsis.installerIcon, "icons/icon.ico");
  assert.equal(tauriConfig.bundle.windows.nsis.uninstallerIcon, "icons/icon.ico");
  assert.match(macBuild, /Baldojnyi Downloader\.app/);
  assert.match(macBuild, /git status --porcelain\)/);
  assert.doesNotMatch(macBuild, /--untracked-files=no/);
  assert.match(releaseGenerator, /Baldojnyi Downloader\.app\.tar\.gz/);
});

test("grants repository write access only to the final draft-release job", async () => {
  const workflow = await readFile(".github/workflows/release.yml", "utf8");
  assert.match(workflow, /^permissions:\r?\n  contents: read$/m);
  assert.match(workflow, /publish-draft:[\s\S]*?permissions:\r?\n      contents: write/);
  assert.equal(workflow.match(/contents: write/g)?.length, 1);
});

test("keeps every current project and public release version in sync", async () => {
  const packageJson = JSON.parse(await readFile("package.json", "utf8"));
  const packageLock = JSON.parse(await readFile("package-lock.json", "utf8"));
  const tauriConfig = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
  const cargoToml = await readFile("src-tauri/Cargo.toml", "utf8");
  const cargoLock = await readFile("src-tauri/Cargo.lock", "utf8");
  const releaseWorkflow = await readFile(".github/workflows/release.yml", "utf8");
  const internalVersionPattern = packageJson.version.replaceAll(".", "\\.");
  const publicVersionPattern = packageJson.releaseVersion.replaceAll(".", "\\.");

  const internalParts = packageJson.version.split(".").map(Number);
  const publicParts = packageJson.releaseVersion.split(".").map(Number);
  assert.equal(internalParts.length, 3);
  assert.equal(publicParts.length, 4);
  assert.ok(internalParts.every(Number.isSafeInteger));
  assert.ok(publicParts.every(Number.isSafeInteger));
  assert.deepEqual(publicParts.slice(0, 2), internalParts.slice(0, 2));
  assert.ok(publicParts[3] >= 0 && publicParts[3] <= 9);
  assert.equal(internalParts[2], publicParts[2] * 10 + publicParts[3]);
  assert.equal(packageLock.version, packageJson.version);
  assert.equal(packageLock.packages[""].version, packageJson.version);
  assert.equal(tauriConfig.version, packageJson.version);
  assert.match(cargoToml, new RegExp(`^version = "${internalVersionPattern}"$`, "m"));
  assert.match(cargoLock, new RegExp(`name = "yt-dlp-desktop"\\r?\\nversion = "${internalVersionPattern}"`));
  assert.match(releaseWorkflow, new RegExp(`default: "v${publicVersionPattern}"`));

  for (const file of ["RELEASE_NOTES.md", "docs/START_HERE_UK.md", "docs/GITHUB_RELEASE_HANDOFF_UK.md", "READMEAI"]) {
    assert.match(await readFile(file, "utf8"), new RegExp(publicVersionPattern), `${file} must mention ${packageJson.releaseVersion}`);
  }
});

test("builds a combined macOS and Windows updater manifest", () => {
  const manifest = buildReleaseManifest({
    assetNames: [
      expected.macUpdater,
      `${expected.macUpdater}.sig`,
      expected.macDmg,
      expected.windowsInstaller,
      `${expected.windowsInstaller}.sig`,
    ],
    signatures: {
      [expected.macUpdater]: "mac-signature",
      [expected.windowsInstaller]: "windows-signature",
    },
    repository: "BaldojniSylyUkrainy/yt-dlp-BD",
    releaseVersion,
    updaterVersion,
    notes: "Test release",
    pubDate: "2026-07-26T00:00:00.000Z",
    requireAll: true,
  });

  assert.equal(manifest.version, updaterVersion);
  assert.equal(manifest.platforms["darwin-aarch64"].signature, "mac-signature");
  assert.equal(manifest.platforms["windows-x86_64"].signature, "windows-signature");
  assert.deepEqual(
    manifest.platforms["windows-x86_64-nsis"],
    manifest.platforms["windows-x86_64"],
  );
  assert.match(
    manifest.platforms["windows-x86_64"].url,
    /releases\/download\/v0\.2\.0\.0\/BaldojnyiDownloader-0\.2\.0\.0-Windows-x64-Setup\.exe$/,
  );
  assert.match(
    manifest.platforms["darwin-aarch64"].url,
    /releases\/download\/v0\.2\.0\.0\/BaldojnyiDownloader-0\.2\.0\.0-Mac-Apple-Silicon-AutoUpdate\.app\.tar\.gz$/,
  );
});

test("fails closed when a required updater signature is missing", () => {
  assert.throws(
    () =>
      buildReleaseManifest({
        assetNames: [expected.macUpdater, expected.macDmg, expected.windowsInstaller],
        signatures: {
          [expected.macUpdater]: "mac-signature",
        },
        repository: "BaldojniSylyUkrainy/yt-dlp-BD",
        releaseVersion,
        updaterVersion,
        notes: "Test release",
        pubDate: "2026-07-26T00:00:00.000Z",
        requireAll: true,
      }),
    /Missing or empty updater signature/,
  );
});

test("requires meaningful versioned release notes", () => {
  const notes = `# yt-dlp BD ${releaseVersion}\n\n## Що нового\n\n- Додано зрозумілий опис важливих змін для користувачів застосунку.`;
  assert.equal(validateReleaseNotes(notes, releaseVersion), notes);
  assert.throws(() => validateReleaseNotes("TODO", releaseVersion), /meaningful/);
  assert.throws(
    () => validateReleaseNotes(notes.replace(releaseVersion, "9.9.9.9"), releaseVersion),
    /must mention release version/,
  );
});
