// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";

const script = readFileSync("src-tauri/resources/media_browser_guide.js", "utf8");
afterEach(() => { document.body.innerHTML = ""; document.head.innerHTML = ""; document.getElementById("baldoj-browser-guide")?.remove(); Reflect.deleteProperty(window, "__baldojGuide"); vi.restoreAllMocks(); });
describe("browser fallback guidance", () => {
  it("explains play, changes to found, and returns without granting remote IPC", () => {
    const original = HTMLElement.prototype.attachShadow;
    let shadow: ShadowRoot | undefined;
    vi.spyOn(HTMLElement.prototype, "attachShadow").mockImplementation(function(this: HTMLElement, init) { shadow = original.call(this, init); return shadow; });
    const location = {href: "https://example.com"};
    new Function("window", "document", "location", "ResizeObserver", script)(window, document, location, class {observe() {}});
    document.dispatchEvent(new Event("DOMContentLoaded"));
    expect(shadow?.textContent).toContain("Натисніть ▶");
    window.dispatchEvent(new Event("baldoj-source-found"));
    expect(shadow?.textContent).toContain("Джерело знайдено");
    window.dispatchEvent(new Event("baldoj-session-expired"));
    expect(shadow?.textContent).toContain("15 хвилин пошуку минули");
    expect(shadow?.textContent).not.toContain("Джерело знайдено");
    shadow?.querySelector("button")?.click();
    expect(location.href).toBe("baldoj-media://return/");
  });
  it("never injects page colors, filters or a force-dark toggle", () => {
    expect(script).not.toMatch(/invert\(|hue-rotate\(|dark\.disabled|Оригінальні кольори/);
    expect(script).not.toMatch(/body\s*\{[^}]*background/);
    expect(script).toContain("attachShadow");
    expect(script).not.toContain("__TAURI");
  });
});
