import { describe, expect, it } from "vitest";
import { maintenanceDue, maintenanceRetryAt, RETRY_DELAY_MS } from "./maintenance";

describe("runtime maintenance scheduling", () => {
  const now = 100 * RETRY_DELAY_MS;
  const storage = (values: Record<string, string>) => ({ getItem: (key: string) => values[key] || null });
  it("backs off a failed attempt across restarts", () => {
    expect(maintenanceDue(storage({ "nightly.attempt": String(now) }), "nightly", 24 * RETRY_DELAY_MS, now + 1000)).toBe(false);
    expect(maintenanceDue(storage({ "nightly.attempt": String(now) }), "nightly", 24 * RETRY_DELAY_MS, now + RETRY_DELAY_MS)).toBe(true);
  });
  it("honors server retry time but bounds untrusted delays", () => {
    expect(maintenanceRetryAt(`[retry_at=${(now + 2 * RETRY_DELAY_MS) / 1000}]`, now)).toBe(now + 2 * RETRY_DELAY_MS);
    expect(maintenanceRetryAt("network error", now)).toBe(now + RETRY_DELAY_MS);
    expect(maintenanceRetryAt("[retry_at=9999999999999]", now)).toBe(now + 24 * RETRY_DELAY_MS);
  });
  it("doesn't recheck a successful daily update", () => {
    expect(maintenanceDue(storage({ nightly: String(now) }), "nightly", 24 * RETRY_DELAY_MS, now + RETRY_DELAY_MS)).toBe(false);
  });
});
