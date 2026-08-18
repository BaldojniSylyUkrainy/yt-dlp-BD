import assert from "node:assert/strict";
import test from "node:test";
import {
  checksumForAsset,
  macFfmpegAssetUrl,
  pinRuntimeReleaseAssets,
  runtimeAssetNames,
  singleChecksum,
  validateRuntimeManifest,
  YT_DLP_RELEASE_SOURCE,
} from "./generate-runtime-manifest.mjs";

const hash = "a".repeat(64);

test("uses yt-dlp's officially recommended nightly channel", () => {
  assert.deepEqual(YT_DLP_RELEASE_SOURCE, {
    owner: "yt-dlp",
    repository: "yt-dlp-nightly-builds",
  });
});

test("uses a stable versioned name for the redistributed Windows FFmpeg runtime", () => {
  assert.deepEqual(runtimeAssetNames("0.6.1.0"), {
    windowsFfmpeg: "BaldojnyiDownloader-0.6.1.0-Runtime-Windows-x64-FFmpeg.zip",
  });
});

test("pins Windows FFmpeg to this app's immutable versioned release asset", () => {
  const manifest = {
    releaseVersion: "0.6.1.0",
    platforms: {
      "windows-x86_64": {
        ffmpeg: { archive: { url: "https://example.invalid/moving-latest.zip" } },
      },
    },
  };
  pinRuntimeReleaseAssets(manifest, "BaldojniSylyUkrainy/yt-dlp-BD", "0.6.1.0");
  assert.equal(
    manifest.platforms["windows-x86_64"].ffmpeg.archive.url,
    "https://github.com/BaldojniSylyUkrainy/yt-dlp-BD/releases/download/v0.6.1.0/BaldojnyiDownloader-0.6.1.0-Runtime-Windows-x64-FFmpeg.zip",
  );
});

test("selects exactly the requested checksum asset", () => {
  assert.equal(checksumForAsset(`${"b".repeat(64)}  other.zip\n${hash} *wanted.zip\n`, "wanted.zip"), hash);
  assert.throws(() => checksumForAsset(`${hash}  other.zip\n`, "wanted.zip"), /exactly one/);
});

test("requires one unambiguous standalone checksum", () => {
  assert.equal(singleChecksum(`SHA256: ${hash}\n`, "fixture"), hash);
  assert.throws(() => singleChecksum(`${hash} ${"b".repeat(64)}`, "fixture"), /exactly one/);
});

test("selects FFmpeg only from the stable Apple Silicon section", () => {
  const page = `<a href="/download/macos/arm64/nightly/ffmpeg.zip">nightly</a>
    <h2>Download Release Build</h2>
    <a href="/download/macos/arm64/7.1/ffmpeg.zip">release</a>`;
  assert.equal(
    macFfmpegAssetUrl(page, "ffmpeg.zip"),
    "https://ffmpeg.martin-riedl.de/download/macos/arm64/7.1/ffmpeg.zip",
  );
});

test("runtime manifest requires both shipped targets and complete FFmpeg layouts", () => {
  const asset = (version, url) => ({ version, url, sha256: hash });
  const manifest = {
    schemaVersion: 1,
    releaseVersion: "0.6.1.0",
    generatedAt: "2026-08-09T00:00:00.000Z",
    platforms: {
      "darwin-aarch64": {
        ytDlp: asset("2026.08.09", "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.09/yt-dlp_macos"),
        deno: asset("v2.4.0", "https://github.com/denoland/deno/releases/download/v2.4.0/deno-aarch64-apple-darwin.zip"),
        ffmpeg: {
          version: "7.1",
          archive: null,
          ffmpeg: asset("7.1", "https://ffmpeg.martin-riedl.de/download/macos/arm64/7.1/ffmpeg.zip"),
          ffprobe: asset("7.1", "https://ffmpeg.martin-riedl.de/download/macos/arm64/7.1/ffprobe.zip"),
        },
      },
      "windows-x86_64": {
        ytDlp: asset("2026.08.09", "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.09/yt-dlp.exe"),
        deno: asset("v2.4.0", "https://github.com/denoland/deno/releases/download/v2.4.0/deno-x86_64-pc-windows-msvc.zip"),
        ffmpeg: {
          version: "latest",
          archive: asset("latest", "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"),
          ffmpeg: null,
          ffprobe: null,
        },
      },
    },
  };
  assert.equal(validateRuntimeManifest(manifest), manifest);
  const missingReleaseVersion = structuredClone(manifest);
  delete missingReleaseVersion.releaseVersion;
  assert.throws(() => validateRuntimeManifest(missingReleaseVersion), /releaseVersion/);
  delete manifest.platforms["windows-x86_64"];
  assert.throws(() => validateRuntimeManifest(manifest), /windows-x86_64/);
});
