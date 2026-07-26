import { describe, expect, it } from "vitest";
import {
  applyDownloadEvent,
  isCurrentProbe,
  isLikelyMultiItemUrl,
  preflightAllowsStart,
  preflightConfidenceLabel,
  runtimeInstallCommand,
  type DownloadEvent,
  type Job,
} from "./App";

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
    ...overrides,
  };
}

describe("download event state", () => {
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
