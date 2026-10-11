import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

test("Tauri URL patterns use maintained ICU instead of unmaintained rust-unic", async () => {
  const lock = await readFile("src-tauri/Cargo.lock", "utf8");
  const packages = lock.split(/\[\[package\]\]/);
  assert.doesNotMatch(lock, /^name = "unic-[^"]+"/m);
  const patterns = packages.filter((entry) => /^name = "urlpattern"$/m.test(entry));
  assert.ok(patterns.length > 0, "expected Tauri URLPattern dependency");
  for (const entry of patterns) {
    assert.match(entry, /"icu_properties(?: [^"]+)?"/);
  }
});
