import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { runInNewContext } from "node:vm";

test("main-window thumbnails cannot bypass the guarded raster cache", async () => {
  const config = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
  const images = config.app.security.csp.split(';').find(rule => rule.trim().startsWith('img-src'));
  assert.ok(images);
  assert.doesNotMatch(images, /https:|\*/);
  const thumbnail = await readFile("src/HistoryThumbnail.tsx", "utf8");
  assert.doesNotMatch(thumbnail, /entry\.thumbnail/);
  assert.match(thumbnail, /convertFileSrc/);
});

test("browser capture uses only the native-intercepted custom scheme and excludes blob sources", async () => {
  const script = await readFile("src-tauri/resources/media_capture.js", "utf8");
  let scan;
  let flush;
  const context = {
    URL, Set, JSON, encodeURIComponent,
    location: {href: "https://example.com/watch"},
    document: {querySelectorAll: () => [{src: "blob:private"}, {src: "https://cdn.example.com/video.mp4?signature=private"}]},
    PerformanceObserver: class { observe() {} },
    XMLHttpRequest: class { send() {} },
    setInterval: fn => { scan = fn; },
    setTimeout: fn => { flush = fn; }, clearTimeout() {},
  };
  context.window = context;
  runInNewContext(script, context);
  scan(); flush();
  const navigation = new URL(context.location.href);
  assert.equal(navigation.protocol, "baldoj-media:");
  const payload = JSON.parse(navigation.searchParams.get("payload"));
  assert.deepEqual(payload.urls, ["https://cdn.example.com/video.mp4?signature=private"]);
  assert.equal(script.includes("document.cookie"), false);
});

test("remote discovery window has no main-window capabilities or direct downloads", async () => {
  const capabilities = JSON.parse(await readFile("src-tauri/capabilities/default.json", "utf8"));
  assert.deepEqual(capabilities.windows, ["main"]);
  assert.equal(capabilities.remote, undefined);
  const backend = await readFile("src-tauri/src/discovery.rs", "utf8");
  assert.match(backend, /\.incognito\(true\)/);
  assert.match(backend, /\.on_download\(\|_, _\| false\)/);
  assert.match(backend, /NewWindowResponse::Deny/);
  assert.match(backend, /\.theme\(Some\(tauri::Theme::Dark\)\)/);
  assert.match(backend, /media_browser_guide\.js/);
  assert.match(backend, /url\.host_str\(\) == Some\("return"\)/);
});
