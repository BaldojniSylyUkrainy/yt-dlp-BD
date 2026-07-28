import { describe, expect, it } from "vitest";
import {
  appendQueueUrls,
  canEditQueueItem,
  createQueueItem,
  nextPendingQueueItem,
  normalizeHttpUrl,
  parseQueueStorage,
  QUEUE_LIMIT,
  queueProgress,
  queueHasActiveProcess,
  queuePreventsOtherWork,
  resetQueueItemsForRetry,
  type QueueSettings,
} from "./queue";

const settings: QueueSettings = {
  outputDir: "/Downloads",
  mode: "video",
  quality: "best",
  audioFormat: "mp3",
  subtitles: false,
  multiItem: false,
  cookiesBrowser: null,
};

describe("batch queue input", () => {
  it("accepts arbitrary HTTP and HTTPS hosts without a service allowlist", () => {
    expect(normalizeHttpUrl("example.org/media/1")).toBe("https://example.org/media/1");
    expect(normalizeHttpUrl("ftp://example.org/file")).toBeNull();
  });

  it("splits multiline paste and enforces the 50 URL limit", () => {
    let id = 0;
    const raw = Array.from({ length: QUEUE_LIMIT + 3 }, (_, index) => `https://example.org/${index}`).join("\n");
    const result = appendQueueUrls([], raw, () => `item-${++id}`);
    expect(result.items).toHaveLength(QUEUE_LIMIT);
    expect(result.rejected).toBe(3);
  });
});

describe("batch queue lifecycle", () => {
  it("counts terminal rows and selects only a runnable row", () => {
    const completed = { ...createQueueItem("https://one.example", "one"), status: "completed" as const };
    const failed = { ...createQueueItem("https://two.example", "two"), status: "failed" as const };
    const pending = createQueueItem("https://three.example", "three");
    expect(queueProgress([completed, failed, pending])).toEqual({ done: 2, total: 3, percent: 67 });
    expect(nextPendingQueueItem([completed, failed, pending])?.id).toBe("three");
  });

  it("rebuilds only failed and interrupted rows for retry", () => {
    const completed = { ...createQueueItem("https://one.example", "one"), status: "completed" as const };
    const failed = { ...createQueueItem("https://two.example", "two"), status: "failed" as const, attempt: 2 };
    const interrupted = { ...createQueueItem("https://three.example", "three"), status: "interrupted" as const };
    const retry = resetQueueItemsForRetry([completed, failed, interrupted]);
    expect(retry.map((item) => item.id)).toEqual(["two", "three"]);
    expect(retry.every((item) => item.status === "pending")).toBe(true);
    expect(retry[0].attempt).toBe(2);
  });

  it("restores an active queue as paused and marks its active row interrupted", () => {
    const active = { ...createQueueItem("https://example.org/video", "active"), status: "downloading" as const, jobId: "job" };
    const restored = parseQueueStorage(JSON.stringify({
      version: 1,
      status: "running",
      settings,
      items: [active],
      activeItemId: active.id,
      createdAt: "2026-07-28T00:00:00.000Z",
      updatedAt: "2026-07-28T00:01:00.000Z",
    }), settings);
    expect(restored?.status).toBe("paused");
    expect(restored?.activeItemId).toBeNull();
    expect(restored?.items[0]).toMatchObject({ status: "interrupted", jobId: null });
  });

  it("preserves an empty draft so destination settings survive a restart", () => {
    const restored = parseQueueStorage(JSON.stringify({
      version: 1,
      status: "draft",
      settings: { ...settings, outputDir: "/Volumes/Archive" },
      items: [],
      activeItemId: null,
      createdAt: "2026-07-28T00:00:00.000Z",
      updatedAt: "2026-07-28T00:01:00.000Z",
    }), settings);
    expect(restored?.items).toEqual([]);
    expect(restored?.settings.outputDir).toBe("/Volumes/Archive");
  });

  it("treats pause-after-current as an active process and blocks competing work", () => {
    const item = { ...createQueueItem("https://example.org/video", "active"), status: "downloading" as const };
    const queue = {
      version: 1 as const,
      status: "paused" as const,
      settings,
      items: [item],
      activeItemId: item.id,
      createdAt: "2026-07-28T00:00:00.000Z",
      updatedAt: "2026-07-28T00:01:00.000Z",
    };
    expect(queueHasActiveProcess(queue)).toBe(true);
    expect(queuePreventsOtherWork(queue)).toBe(true);
    expect(canEditQueueItem(queue, item)).toBe(false);
  });

  it("allows only failed rows to be corrected after a completed run", () => {
    const failed = { ...createQueueItem("https://bad.example", "bad"), status: "failed" as const };
    const completed = { ...createQueueItem("https://good.example", "good"), status: "completed" as const };
    const queue = {
      version: 1 as const,
      status: "completed" as const,
      settings,
      items: [failed, completed],
      activeItemId: null,
      createdAt: "2026-07-28T00:00:00.000Z",
      updatedAt: "2026-07-28T00:01:00.000Z",
    };
    expect(canEditQueueItem(queue, failed)).toBe(true);
    expect(canEditQueueItem(queue, completed)).toBe(false);
  });
});
