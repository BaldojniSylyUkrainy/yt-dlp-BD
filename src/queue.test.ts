import { describe, expect, it } from "vitest";
import {
  appendQueueUrls,
  canEditQueueItem,
  commitQueueInput,
  createQueueItem,
  nextPendingQueueItem,
  normalizeHttpUrl,
  parseQueueStorage,
  QUEUE_LIMIT,
  queueProgress,
  queueHasActiveProcess,
  queuePreventsOtherWork,
  resetEntireQueueForReplay,
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

  it("removes TikTok tracking parameters only from canonical video links", () => {
    expect(normalizeHttpUrl("https://www.tiktok.com/@the_best_president_ua/video/7667686431778688274?is_from_webapp=1&sender_device=pc"))
      .toBe("https://www.tiktok.com/@the_best_president_ua/video/7667686431778688274");
    expect(normalizeHttpUrl("https://vm.tiktok.com/ZMexample/?share_app_id=123"))
      .toBe("https://vm.tiktok.com/ZMexample/?share_app_id=123");
  });

  it("splits multiline paste and enforces the 50 URL limit", () => {
    let id = 0;
    const raw = Array.from({ length: QUEUE_LIMIT + 3 }, (_, index) => `https://example.org/${index}`).join("\n");
    const result = appendQueueUrls([], raw, () => `item-${++id}`);
    expect(result.items).toHaveLength(QUEUE_LIMIT);
    expect(result.rejected).toBe(3);
  });

  it("commits the still-focused URL before the same Start action selects work", () => {
    const committed = commitQueueInput(
      null,
      settings,
      "https://example.org/first",
      "2026-07-29T00:00:00.000Z",
      () => "first",
    );
    expect(committed.queue?.items).toHaveLength(1);
    expect(nextPendingQueueItem(committed.queue?.items || [])?.url).toBe("https://example.org/first");
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

  it("includes the active row in the overall queue percentage", () => {
    const completed = { ...createQueueItem("https://one.example", "one"), status: "completed" as const, percent: 100 };
    const active = { ...createQueueItem("https://two.example", "two"), status: "downloading" as const, percent: 50 };
    expect(queueProgress([completed, active])).toEqual({ done: 1, total: 2, percent: 75 });
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

  it("rebuilds every row when the user repeats the whole queue", () => {
    const completed = { ...createQueueItem("https://one.example", "one"), status: "completed" as const, finalSize: 42 };
    const failed = { ...createQueueItem("https://two.example", "two"), status: "failed" as const, attempt: 2 };
    const skipped = { ...createQueueItem("https://three.example", "three"), status: "skipped" as const };
    const replay = resetEntireQueueForReplay([completed, failed, skipped]);
    expect(replay.map((item) => item.id)).toEqual(["one", "two", "three"]);
    expect(replay.every((item) => item.status === "pending" && item.finalSize === 0)).toBe(true);
    expect(replay[1].attempt).toBe(2);
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
