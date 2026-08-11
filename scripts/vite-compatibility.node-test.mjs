import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { build } from "vite";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("production CSS remains compatible with Safari 15.4 on macOS 12.3", async () => {
  const outDir = await mkdtemp(path.join(os.tmpdir(), "yt-dlp-bd-vite-compat-"));

  try {
    await build({
      root: projectRoot,
      logLevel: "silent",
      build: { outDir, emptyOutDir: true },
    });

    const assetDir = path.join(outDir, "assets");
    const cssFiles = (await readdir(assetDir)).filter((name) => name.endsWith(".css"));
    assert.ok(cssFiles.length > 0, "production build must emit CSS");

    const css = (await Promise.all(cssFiles.map((name) => readFile(path.join(assetDir, name), "utf8")))).join("\n");
    assert.doesNotMatch(css, /@media\s*\((?:width|height)\s*[<>]=/u);
    assert.match(css, /@media\s*\(max-width:1020px\)/u);
    assert.match(css, /@media\s*\(max-height:760px\)/u);
  } finally {
    await rm(outDir, { recursive: true, force: true });
  }
});
