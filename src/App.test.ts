import { describe, expect, it } from "vitest";
import {
  applyDownloadEvent,
  applyHistoryFileStatuses,
  applyHistoryThumbnailCache,
  defaultCookieBrowser,
  groupHistoryEntries,
  historyPaths,
  isCurrentProbe,
  isLikelyMultiItemUrl,
  parseHistoryStorage,
  preflightAllowsStart,
  preflightConfidenceLabel,
  runtimeInstallCommand,
  shouldPlayCompletionSound,
  shouldPlayQueueCompletionSound,
  shouldCacheHistoryThumbnail,
  shouldDeleteUnusedHistoryThumbnail,
  UPDATE_CHECK_DELAYS,
  type DownloadEvent,
  type Job,
  type HistoryEntry,
} from "./App";
import { createQueueItem } from "./queue";

const startingJob: Job = {
  id: "job-1",
  url: "https://youtube.com/watch?v=video",
  title: "youtube.com",
  status: "starting",
  percent: 0,
  speed: "—",
  eta: "—",
  message: "Підключення…",
  storage: null,
  playlist: false,
  outputFormat: "MP4",
};

function event(kind: DownloadEvent["kind"], overrides: Partial<DownloadEvent> = {}): DownloadEvent {
  return {
    id: "job-1",
    kind,
    percent: null,
    speed: null,
    eta: null,
    message: null,
    storage: null,
    outputs: null,
    ...overrides,
  };
}

describe("download event state", () => {
  it("plays the completion sound only after a successful job", () => {
    expect(shouldPlayCompletionSound("completed")).toBe(true);
    expect(shouldPlayCompletionSound("failed")).toBe(false);
    expect(shouldPlayCompletionSound("cancelled")).toBe(false);
  });

  it("plays one queue sound only when the batch produced at least one finished file", () => {
    expect(shouldPlayQueueCompletionSound([
      { ...createQueueItem("https://example.com/one"), status: "failed" },
      { ...createQueueItem("https://example.com/two"), status: "skipped" },
    ])).toBe(false);
    expect(shouldPlayQueueCompletionSound([
      { ...createQueueItem("https://example.com/one"), status: "failed" },
      { ...createQueueItem("https://example.com/two"), status: "completed" },
    ])).toBe(true);
  });

  it("applies progress even when it is the first event after job creation", () => {
    const next = applyDownloadEvent(startingJob, event("progress", {
      percent: 12.5,
      speed: "2 MiB/s",
      eta: "00:42",
      message: "Назва відео",
    }));
    expect(next).toMatchObject({
      status: "downloading",
      percent: 12.5,
      speed: "2 MiB/s",
      eta: "00:42",
      title: "Назва відео",
    });
  });

  it("never leaves an immediate terminal error in starting state", () => {
    const next = applyDownloadEvent(startingJob, event("failed", { message: "Посилання недоступне" }));
    expect(next).toMatchObject({ status: "failed", message: "Посилання недоступне" });
  });

  it("opens the auth flow for an immediate authentication event", () => {
    const next = applyDownloadEvent(startingJob, event("auth_required"));
    expect(next).toMatchObject({ status: "auth_required", message: "Потрібен вхід через браузер" });
  });

  it("removes a cancelled job", () => {
    expect(applyDownloadEvent(startingJob, event("cancelled"))).toBeNull();
  });
});

describe("download history grouping", () => {
  const entry = (id: string, date: Date): HistoryEntry => ({
    id,
    sourceUrl: "https://example.com/video",
    title: `Відео ${id}`,
    thumbnail: null,
    cachedThumbnailPath: null,
    uploader: null,
    extractor: "Example",
    size: 1024,
    downloadedAt: date.toISOString(),
    path: `/Downloads/${id}.mp4`,
    available: true,
    settings: {
      outputDir: "/Downloads",
      mode: "video",
      quality: "best",
      audioFormat: "mp3",
      subtitles: false,
      playlist: false,
    },
  });

  it("sorts newest downloads first and groups them by local calendar day", () => {
    const now = new Date(2026, 6, 27, 12, 0, 0);
    const yesterday = new Date(2026, 6, 26, 18, 0, 0);
    const morning = new Date(2026, 6, 27, 8, 0, 0);
    const groups = groupHistoryEntries([entry("old", yesterday), entry("new", morning)], now);
    expect(groups.map((group) => group.label)).toEqual(["Сьогодні", "Вчора"]);
    expect(groups[0].items[0].id).toBe("new");
  });

  it("polls replacement paths at the 500-entry limit without rebuilding unchanged state", () => {
    const full = Array.from({ length: 500 }, (_, index) => entry(`old-${index}`, new Date(2026, 6, 27, 8, 0, index)));
    const replacement = entry("replacement", new Date(2026, 6, 27, 12, 0, 0));
    const current = [replacement, ...full.slice(0, 499)];
    expect(current).toHaveLength(full.length);
    expect(historyPaths(current)).toContain("/Downloads/replacement.mp4");
    expect(historyPaths(current)).not.toContain("/Downloads/old-499.mp4");

    const unchanged = applyHistoryFileStatuses(current, current.map((item) => ({
      path: item.path,
      available: item.available,
      size: item.size,
    })));
    expect(unchanged).toBe(current);

    const changed = applyHistoryFileStatuses(current, [{
      path: replacement.path,
      available: false,
      size: null,
    }]);
    expect(changed).not.toBe(current);
    expect(changed[0]).toMatchObject({ id: "replacement", available: false });
  });

  it("caches only completed jobs and does not resurrect history cleared during caching", () => {
    const outputs = [{ path: "/Downloads/video.mp4", size: 1024 }];
    expect(shouldCacheHistoryThumbnail("completed", outputs, "https://example.com/thumb.jpg")).toBe(true);
    expect(shouldCacheHistoryThumbnail("failed", outputs, "https://example.com/thumb.jpg")).toBe(true);
    expect(shouldCacheHistoryThumbnail("cancelled", outputs, "https://example.com/thumb.jpg")).toBe(true);
    expect(shouldCacheHistoryThumbnail("failed", null, "https://example.com/thumb.jpg")).toBe(false);
    expect(shouldCacheHistoryThumbnail("completed", outputs, null)).toBe(false);

    const populated = [entry("completed", new Date(2026, 6, 27, 12, 0, 0))];
    expect(applyHistoryThumbnailCache(populated, new Set(["completed"]), "/cache/thumb.jpg")[0].cachedThumbnailPath)
      .toBe("/cache/thumb.jpg");
    const cleared: HistoryEntry[] = [];
    expect(applyHistoryThumbnailCache(cleared, new Set(["completed"]), "/cache/thumb.jpg")).toBe(cleared);
    expect(shouldDeleteUnusedHistoryThumbnail(cleared, "https://example.com/thumb.jpg", "/cache/thumb.jpg")).toBe(true);
    const sharedThumbnail = [{ ...populated[0], thumbnail: "https://example.com/thumb.jpg" }];
    expect(shouldDeleteUnusedHistoryThumbnail(sharedThumbnail, "https://example.com/thumb.jpg", "/cache/thumb.jpg")).toBe(false);
  });

  it("rejects malformed storage and invalid required fields without losing valid neighbours", () => {
    expect(parseHistoryStorage("{broken")).toEqual([]);
    expect(parseHistoryStorage(JSON.stringify({ entry: "not an array" }))).toEqual([]);

    const valid = entry("valid", new Date(2026, 6, 27, 12, 0, 0));
    const invalidDate = { ...valid, id: "bad-date", downloadedAt: "not-a-date" };
    const missingSource = { ...valid, id: "missing-source", sourceUrl: undefined };
    const parsed = parseHistoryStorage(JSON.stringify([invalidDate, valid, missingSource]));
    expect(parsed.map((item) => item.id)).toEqual(["valid"]);
  });

  it("migrates partial settings and normalizes unknown enum values", () => {
    const legacy = {
      ...entry("legacy", new Date(2026, 6, 27, 12, 0, 0)),
      size: "unknown",
      available: "yes",
      settings: {
        outputDir: "",
        mode: "document",
        quality: "8k",
        audioFormat: "flac",
        subtitles: "yes",
      },
    };
    const [parsed] = parseHistoryStorage(JSON.stringify([legacy]));
    expect(parsed).toMatchObject({ size: 0, available: true });
    expect(parsed.settings).toEqual({
      outputDir: "/Downloads",
      mode: "video",
      quality: "best",
      audioFormat: "mp3",
      subtitles: false,
      playlist: false,
    });
  });
});

describe("playlist intent detection", () => {
  it("recognizes a watch URL containing a playlist without choosing the intent for the user", () => {
    expect(isLikelyMultiItemUrl("https://youtube.com/watch?v=VIDEO&list=PLAYLIST")).toBe(true);
    expect(startingJob.playlist).toBe(false);
  });

  it("does not treat an ordinary watch URL as a collection", () => {
    expect(isLikelyMultiItemUrl("https://youtube.com/watch?v=VIDEO")).toBe(false);
  });
});

describe("preflight and retry guards", () => {
  it("blocks start when preflight reports insufficient space", () => {
    expect(preflightAllowsStart({
      title: "Video",
      itemCount: 1,
      intermediateSize: 2_000,
      finalOutputSize: 13_000,
      protectedReserve: 500,
      requiredSpace: 15_500,
      availableSpace: 10_000,
      confidence: "approximate",
      sufficient: false,
    })).toBe(false);
  });

  it("labels an unknown estimate without inventing a number", () => {
    expect(preflightConfidenceLabel("unknown")).toBe("Розмір частково невідомий");
  });

  it("maps every runtime retry to its own backend command", () => {
    expect(runtimeInstallCommand("ytDlp")).toBe("install_ytdlp");
    expect(runtimeInstallCommand("ffmpeg")).toBe("install_ffmpeg");
    expect(runtimeInstallCommand("deno")).toBe("install_deno");
  });

  it("rejects a stale probe response", () => {
    expect(isCurrentProbe(8, 7)).toBe(false);
    expect(isCurrentProbe(8, 8)).toBe(true);
  });

  it("preserves the selected output format during audio conversion", () => {
    const audioJob = { ...startingJob, outputFormat: "MP3" };
    const next = applyDownloadEvent(audioJob, event("conversion_progress", { percent: 50 }));
    expect(next).toMatchObject({ status: "converting", outputFormat: "MP3", percent: 50 });
  });
});

describe("platform defaults", () => {
  it("uses Edge instead of Safari on Windows", () => {
    expect(defaultCookieBrowser("windows", null)).toBe("edge");
    expect(defaultCookieBrowser("windows", "safari")).toBe("edge");
    expect(defaultCookieBrowser("windows", "chrome")).toBe("chrome");
  });

  it("keeps Safari as the macOS default", () => {
    expect(defaultCookieBrowser("macos", null)).toBe("safari");
  });
});

describe("application updates", () => {
  it("checks immediately and retries after short startup delays", () => {
    expect(UPDATE_CHECK_DELAYS).toEqual([0, 5_000, 30_000]);
  });
});
