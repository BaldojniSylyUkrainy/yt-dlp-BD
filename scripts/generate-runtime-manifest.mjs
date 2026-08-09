import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";
import { fileURLToPath, pathToFileURL } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, "..");

const MAC_FFMPEG_PAGE = "https://ffmpeg.martin-riedl.de/";
const GITHUB_API = "https://api.github.com/repos";

function githubApiHeaders() {
  const headers = {
    Accept: "application/vnd.github+json",
    "User-Agent": "yt-dlp-BD-release",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  const token = process.env.GITHUB_TOKEN?.trim();
  if (token) headers.Authorization = `Bearer ${token}`;
  return headers;
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  return index === -1 ? null : process.argv[index + 1] || null;
}

export function runtimeAssetNames(releaseVersion) {
  return {
    windowsFfmpeg: `BaldojnyiDownloader-${releaseVersion}-Runtime-Windows-x64-FFmpeg.zip`,
  };
}

export function pinRuntimeReleaseAssets(manifest, repository, releaseVersion) {
  if (!/^[\w.-]+\/[\w.-]+$/u.test(repository)) throw new Error("Invalid GitHub repository");
  if (!/^\d+\.\d+\.\d+\.\d+$/u.test(releaseVersion)) throw new Error("Invalid release version");
  const names = runtimeAssetNames(releaseVersion);
  manifest.platforms["windows-x86_64"].ffmpeg.archive.url =
    `https://github.com/${repository}/releases/download/v${releaseVersion}/${names.windowsFfmpeg}`;
  return manifest;
}

export function checksumForAsset(checksums, assetName) {
  const matches = checksums
    .split(/\r?\n/u)
    .map((line) => line.trim().split(/\s+/u))
    .filter(([hash, filename]) =>
      /^[a-f0-9]{64}$/iu.test(hash || "")
      && (filename || "").replace(/^\*|^\.\//u, "") === assetName)
    .map(([hash]) => hash.toLowerCase());
  if (matches.length !== 1) {
    throw new Error(`Expected exactly one SHA-256 for ${assetName}, found ${matches.length}`);
  }
  return matches[0];
}

export function singleChecksum(text, label) {
  const matches = text.match(/\b[a-f0-9]{64}\b/giu) || [];
  if (matches.length !== 1) {
    throw new Error(`Expected exactly one SHA-256 for ${label}, found ${matches.length}`);
  }
  return matches[0].toLowerCase();
}

export function macFfmpegAssetUrl(page, filename) {
  const marker = "<h2>Download Release Build</h2>";
  const markerIndex = page.indexOf(marker);
  if (markerIndex === -1) {
    throw new Error("macOS FFmpeg service did not expose its stable release section");
  }
  const releaseSection = page.slice(markerIndex + marker.length);
  const matches = [...releaseSection.matchAll(/href="([^"]+)"/gu)]
    .map((match) => match[1])
    .filter((candidate) =>
      candidate.startsWith("/download/macos/arm64/") && candidate.endsWith(filename));
  if (matches.length !== 1) {
    throw new Error(`Expected exactly one stable macOS ${filename}, found ${matches.length}`);
  }
  return new URL(matches[0], MAC_FFMPEG_PAGE).toString();
}

export function validateRuntimeManifest(manifest) {
  if (manifest.schemaVersion !== 1) throw new Error("Unsupported runtime manifest schema");
  if (!Number.isFinite(Date.parse(manifest.generatedAt))) {
    throw new Error("Runtime manifest generatedAt is invalid");
  }
  for (const platform of ["darwin-aarch64", "windows-x86_64"]) {
    const entry = manifest.platforms?.[platform];
    if (!entry) throw new Error(`Runtime manifest is missing ${platform}`);
    for (const component of ["ytDlp", "deno"]) validateAsset(entry[component], component);
    if (!entry.ffmpeg?.version) throw new Error(`${platform} FFmpeg version is missing`);
    if (platform === "darwin-aarch64") {
      validateAsset(entry.ffmpeg.ffmpeg, "macOS FFmpeg");
      validateAsset(entry.ffmpeg.ffprobe, "macOS FFprobe");
      if (entry.ffmpeg.archive != null) throw new Error("macOS FFmpeg must use split archives");
    } else {
      validateAsset(entry.ffmpeg.archive, "Windows FFmpeg");
      if (entry.ffmpeg.ffmpeg != null || entry.ffmpeg.ffprobe != null) {
        throw new Error("Windows FFmpeg must use one combined archive");
      }
    }
  }
  return manifest;
}

function validateAsset(asset, label) {
  if (!asset || typeof asset.version !== "string" || asset.version.trim() === "") {
    throw new Error(`${label} version is missing`);
  }
  if (!/^https:\/\//u.test(asset.url || "")) throw new Error(`${label} URL must use HTTPS`);
  if (!/^[a-f0-9]{64}$/u.test(asset.sha256 || "")) throw new Error(`${label} SHA-256 is invalid`);
}

async function fetchText(url) {
  const response = await fetch(url, {
    headers: { Accept: "application/vnd.github+json", "User-Agent": "yt-dlp-BD-release" },
    redirect: "follow",
  });
  if (!response.ok) throw new Error(`${url} returned HTTP ${response.status}`);
  if (new URL(response.url).protocol !== "https:") throw new Error(`${url} redirected away from HTTPS`);
  return response.text();
}

async function githubLatest(owner, repository) {
  const response = await fetch(`${GITHUB_API}/${owner}/${repository}/releases/latest`, {
    headers: githubApiHeaders(),
  });
  if (!response.ok) throw new Error(`${owner}/${repository} latest release returned HTTP ${response.status}`);
  const release = await response.json();
  const assets = new Map(release.assets.map((asset) => [asset.name, asset.browser_download_url]));
  return {
    version: release.tag_name,
    asset(name) {
      const url = assets.get(name);
      if (!url) throw new Error(`${owner}/${repository} release ${release.tag_name} is missing ${name}`);
      return url;
    },
  };
}

async function githubAsset(release, assetName, checksums) {
  return {
    version: release.version,
    url: release.asset(assetName),
    sha256: checksumForAsset(checksums, assetName),
  };
}

async function sha256File(file) {
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(file)) digest.update(chunk);
  return digest.digest("hex");
}

async function downloadVerifiedAsset(asset, destination) {
  const response = await fetch(asset.url, {
    headers: { "User-Agent": "yt-dlp-BD-release" },
    redirect: "follow",
  });
  if (!response.ok || !response.body) {
    throw new Error(`${asset.url} returned HTTP ${response.status}`);
  }
  if (new URL(response.url).protocol !== "https:") {
    throw new Error(`${asset.url} redirected away from HTTPS`);
  }
  await pipeline(Readable.fromWeb(response.body), createWriteStream(destination, { mode: 0o644 }));
  const actual = await sha256File(destination);
  if (actual !== asset.sha256) {
    throw new Error(`Downloaded ${path.basename(destination)} does not match its upstream SHA-256`);
  }
}

export async function buildRuntimeManifest(now = new Date()) {
  const [ytDlp, deno, windowsFfmpeg, macPage] = await Promise.all([
    githubLatest("yt-dlp", "yt-dlp"),
    githubLatest("denoland", "deno"),
    githubLatest("yt-dlp", "FFmpeg-Builds"),
    fetchText(MAC_FFMPEG_PAGE),
  ]);
  const [ytChecksums, windowsFfmpegChecksums] = await Promise.all([
    fetchText(ytDlp.asset("SHA2-256SUMS")),
    fetchText(windowsFfmpeg.asset("checksums.sha256")),
  ]);

  const macDenoName = "deno-aarch64-apple-darwin.zip";
  const windowsDenoName = "deno-x86_64-pc-windows-msvc.zip";
  const windowsFfmpegName = "ffmpeg-master-latest-win64-gpl.zip";
  const [macDenoChecksum, windowsDenoChecksum] = await Promise.all([
    fetchText(deno.asset(`${macDenoName}.sha256sum`)),
    fetchText(deno.asset(`${windowsDenoName}.sha256sum`)),
  ]);

  const macFfmpegUrl = macFfmpegAssetUrl(macPage, "ffmpeg.zip");
  const macFfprobeUrl = macFfmpegAssetUrl(macPage, "ffprobe.zip");
  const [macFfmpegChecksum, macFfprobeChecksum] = await Promise.all([
    fetchText(`${macFfmpegUrl}.sha256`),
    fetchText(`${macFfprobeUrl}.sha256`),
  ]);
  const macFfmpegVersion = new URL(macFfmpegUrl).pathname.split("/").at(-2);
  if (!macFfmpegVersion) throw new Error("Could not determine macOS FFmpeg version");

  return validateRuntimeManifest({
    schemaVersion: 1,
    generatedAt: now.toISOString(),
    platforms: {
      "darwin-aarch64": {
        ytDlp: await githubAsset(ytDlp, "yt-dlp_macos", ytChecksums),
        deno: {
          version: deno.version,
          url: deno.asset(macDenoName),
          sha256: singleChecksum(macDenoChecksum, macDenoName),
        },
        ffmpeg: {
          version: macFfmpegVersion,
          archive: null,
          ffmpeg: {
            version: macFfmpegVersion,
            url: macFfmpegUrl,
            sha256: singleChecksum(macFfmpegChecksum, "macOS FFmpeg"),
          },
          ffprobe: {
            version: macFfmpegVersion,
            url: macFfprobeUrl,
            sha256: singleChecksum(macFfprobeChecksum, "macOS FFprobe"),
          },
        },
      },
      "windows-x86_64": {
        ytDlp: await githubAsset(ytDlp, "yt-dlp.exe", ytChecksums),
        deno: {
          version: deno.version,
          url: deno.asset(windowsDenoName),
          sha256: singleChecksum(windowsDenoChecksum, windowsDenoName),
        },
        ffmpeg: {
          version: windowsFfmpeg.version,
          archive: await githubAsset(windowsFfmpeg, windowsFfmpegName, windowsFfmpegChecksums),
          ffmpeg: null,
          ffprobe: null,
        },
      },
    },
  });
}

async function main() {
  const output = path.resolve(argumentValue("--output") || path.join(projectDir, "runtime-components.json"));
  const manifest = await buildRuntimeManifest();
  const releaseVersion = argumentValue("--release-version");
  const assetsDirValue = argumentValue("--assets-dir");
  if ((releaseVersion == null) !== (assetsDirValue == null)) {
    throw new Error("--release-version and --assets-dir must be provided together");
  }
  if (releaseVersion && assetsDirValue) {
    const releaseConfig = JSON.parse(
      await readFile(path.join(projectDir, "release.config.json"), "utf8"),
    );
    const repository = process.env.GITHUB_REPOSITORY || releaseConfig.githubRepository;
    const assetsDir = path.resolve(assetsDirValue);
    const names = runtimeAssetNames(releaseVersion);
    await mkdir(assetsDir, { recursive: true });
    const windowsFfmpeg = manifest.platforms["windows-x86_64"].ffmpeg.archive;
    await downloadVerifiedAsset(windowsFfmpeg, path.join(assetsDir, names.windowsFfmpeg));
    pinRuntimeReleaseAssets(manifest, repository, releaseVersion);
    validateRuntimeManifest(manifest);
  }
  await writeFile(output, `${JSON.stringify(manifest, null, 2)}\n`, { mode: 0o644 });
  const parsed = JSON.parse(await readFile(output, "utf8"));
  validateRuntimeManifest(parsed);
  console.log(`Runtime manifest generated: ${output}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
