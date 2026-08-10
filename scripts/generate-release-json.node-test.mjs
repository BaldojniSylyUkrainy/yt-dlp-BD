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

test("scopes protected signing secrets to only the steps that need them", async () => {
  const workflow = await readFile(".github/workflows/release.yml", "utf8");
  for (const job of ["windows", "macos", "runtime-manifest"]) {
    const jobHeader = workflow.match(new RegExp(`^  ${job}:([\\s\\S]*?)^    steps:`, "m"))?.[1] || "";
    assert.doesNotMatch(jobHeader, /secrets\.(?:TAURI_SIGNING|APPLE_)/u, `${job} exposes signing secrets at job scope`);
  }
  assert.match(workflow, /Build signed NSIS installer[\s\S]*?TAURI_SIGNING_PRIVATE_KEY:/u);
  assert.match(workflow, /Build, notarize, staple, and verify macOS artifacts[\s\S]*?TAURI_SIGNING_PRIVATE_KEY:/u);
  assert.match(workflow, /Sign and verify runtime manifest outputs[\s\S]*?TAURI_SIGNING_PRIVATE_KEY:/u);
});

test("removes Apple signing files before any external upload action", async () => {
  const workflow = await readFile(".github/workflows/release.yml", "utf8");
  const cleanup = workflow.indexOf("- name: Remove temporary signing material");
  const macUpload = workflow.indexOf("- uses: actions/upload-artifact@", cleanup);
  assert.ok(cleanup > workflow.indexOf("- name: Build, notarize, staple, and verify macOS artifacts"));
  assert.ok(macUpload > cleanup, "macOS upload must run only after signing-material cleanup");
  assert.ok(workflow.indexOf("- name: Verify expected macOS release files", cleanup) > cleanup);
});

test("uses Node 24 artifact actions in both platform release jobs", async () => {
  const workflow = await readFile(".github/workflows/release.yml", "utf8");
  assert.equal(workflow.match(/actions\/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a/g)?.length, 3);
  assert.equal(workflow.match(/actions\/download-artifact@37930b1c2abaa49bbe596cd826c3c89aef350131/g)?.length, 1);
  assert.doesNotMatch(workflow, /actions\/(?:upload|download)-artifact@v4/);
});

test("keeps Dependabot updates in one monthly non-major maintenance PR", async () => {
  const config = await readFile(".github/dependabot.yml", "utf8");
  assert.match(config, /multi-ecosystem-groups:\r?\n  monthly-maintenance:/);
  assert.match(config, /schedule:\r?\n      interval: monthly/);
  assert.equal(config.match(/multi-ecosystem-group: monthly-maintenance/g)?.length, 3);
  assert.equal(config.match(/version-update:semver-major/g)?.length, 3);
  assert.doesNotMatch(config, /interval: weekly/);
});

test("pins every external GitHub Action to an immutable full commit SHA", async () => {
  for (const file of [".github/workflows/release.yml", ".github/workflows/security.yml"]) {
    const workflow = await readFile(file, "utf8");
    const uses = [...workflow.matchAll(/^\s*- uses:\s+([^\s#]+)/gmu)].map((match) => match[1]);
    assert.ok(uses.length > 0, `${file} must contain actions`);
    for (const action of uses) {
      assert.match(action, /^[\w.-]+\/[\w.-]+(?:\/[\w.-]+)?@[a-f0-9]{40}$/u, `${action} in ${file} must be pinned`);
    }
  }
});

test("release publishes a signed runtime component manifest", async () => {
  const workflow = await readFile(".github/workflows/release.yml", "utf8");
  assert.match(workflow, /runtime-manifest:/);
  assert.match(workflow, /tauri signer sign release-assets\/runtime-components\.json/);
  assert.match(workflow, /needs: \[windows, macos, runtime-manifest\]/);
  assert.match(workflow, /test -s release-assets\/runtime-components\.json\.sig/);
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
