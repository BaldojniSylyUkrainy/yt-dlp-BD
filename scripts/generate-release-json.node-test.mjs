import assert from "node:assert/strict";
import test from "node:test";
import {
  buildReleaseManifest,
  expectedAssetNames,
} from "./generate-release-json.mjs";

const releaseVersion = "0.2.0.0";
const updaterVersion = "0.2.0";
const expected = expectedAssetNames(releaseVersion);

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
    /releases\/download\/v0\.2\.0\.0\/yt-dlp-BD_0\.2\.0\.0_windows-x86_64-setup\.exe$/,
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
