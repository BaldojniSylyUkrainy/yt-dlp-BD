// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import { createQueueItem, QUEUE_STORAGE_KEY } from "./queue";

const events = vi.hoisted(() => new Map<string, (event: {payload: unknown}) => void>());
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), convertFileSrc: (p: string) => p }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name, callback) => { events.set(name, callback); return () => events.delete(name); }) }));
vi.mock("@tauri-apps/api/path", () => ({ downloadDir: async () => "/Downloads" }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn(), revealItemInDir: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: async () => null }));

beforeEach(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value), removeItem: (key: string) => storage.delete(key), clear: () => storage.clear() });
  for (const key of ["runtimeUpdate.ytDlpNightly.v1", "runtimeUpdate.ffmpeg", "runtimeUpdate.deno"]) localStorage.setItem(key, String(Date.now()));
  const component = {installed: true, managed: true, version: "test", path: "/runtime/test"};
  vi.mocked(invoke).mockImplementation(async (command) => {
    switch (command) {
      case "runtime_status": return {ytDlp: component, ffmpeg: component, deno: component, runtimeDir: "/runtime", platform: "macos"} as never;
      case "folder_free_space": return 100000000000 as never;
      case "preflight_download": return {sufficient: true, availableSpace: 100000000000, requiredSpace: null, finalOutputSize: null} as never;
      case "interrupted_work": case "inspect_history_files": return [] as never;
      case "take_app_close_request": return null as never;
      case "download_diagnostics": return "Очищений журнал тестового завантаження" as never;
      default: return undefined as never;
    }
  });
});
afterEach(() => { cleanup(); vi.clearAllMocks(); vi.unstubAllGlobals(); events.clear(); });

it("review: history must not load an uncached untrusted thumbnail directly", async () => {
  localStorage.setItem("downloadHistory.v1", JSON.stringify([{
    id: "review-thumbnail", sourceUrl: "https://example.com/video", title: "Thumbnail review",
    thumbnail: "https://127.0.0.1:23971/thumb.jpg", cachedThumbnailPath: null,
    uploader: null, extractor: "example", size: 123, downloadedAt: new Date().toISOString(),
    path: "/Downloads/video.mp4", available: true,
    settings: {outputDir: "/Downloads", mode: "video", quality: "best", audioFormat: "mp3", subtitles: false, playlist: false}
  }]));
  render(<App/>);
  fireEvent.click(screen.getByRole("button", {name: /Історія/i}));
  await screen.findByText("Thumbnail review");
  const unsafeImage = document.querySelector('img[src="https://127.0.0.1:23971/thumb.jpg"]');
  expect(unsafeImage).toBeNull();
});

it("review: a history retry must await destination and never start the old draft", async () => {
  localStorage.setItem("downloadHistory.v1", JSON.stringify([{
    id: "history-review", sourceUrl: "https://example.com/wanted-history", title: "Review history",
    thumbnail: null, cachedThumbnailPath: null, uploader: null, extractor: "example",
    size: 1024, downloadedAt: new Date().toISOString(), path: "/OldTarget/video.mp4", available: true,
    settings: {outputDir: "/OldTarget", mode: "audio", quality: "720", audioFormat: "wav", subtitles: false, playlist: false}
  }]));
  const original = vi.mocked(invoke).getMockImplementation()!;
  let releaseFolder!: (value: number) => void;
  const folderPending = new Promise<number>(resolve => { releaseFolder = resolve; });
  vi.mocked(invoke).mockImplementation((command, args) => {
    if (command === "folder_free_space" && (args as {path?: string})?.path === "/OldTarget") return folderPending as never;
    return original(command, args);
  });
  render(<App/>);
  const input = screen.getByPlaceholderText(/https:\/\/youtube/);
  fireEvent.change(input, {target: {value: "https://example.com/old-draft"}});
  await waitFor(() => expect((screen.getByRole("button", {name: /^Завантажити$/}) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button", {name: /Історія/i}));
  fireEvent.click(await screen.findByRole("button", {name: "Завантажити ще раз"}));
  await new Promise(resolve => setTimeout(resolve, 100));
  const premature = vi.mocked(invoke).mock.calls.filter(([name]) => name === "start_download");
  releaseFolder(100000000000);
  expect(premature).toHaveLength(0);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_download", expect.objectContaining({request: expect.objectContaining({url: "https://example.com/wanted-history", mode: "audio", audioFormat: "wav", outputDir: "/OldTarget"})})));
});

describe("single-download lifecycle", () => {
  it("cancels deferred maintenance when the app view unmounts", async () => {
    const scheduled = vi.spyOn(window, "setTimeout");
    const cleared = vi.spyOn(window, "clearTimeout");
    try {
      const view = render(<App/>);
      await waitFor(() => expect(scheduled.mock.calls.some(([, delay]) => delay === 800)).toBe(true));
      const index = scheduled.mock.calls.findIndex(([, delay]) => delay === 800);
      const id = scheduled.mock.results[index].value;
      view.unmount();
      expect(cleared).toHaveBeenCalledWith(id);
    } finally { scheduled.mockRestore(); cleared.mockRestore(); }
  });
  async function startSingle(url = "https://example.com/video") {
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/youtube/), {target: {value: url}});
    const button = screen.getByRole("button", {name: /^Завантажити$/});
    await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(button);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_download", expect.anything()));
    return (vi.mocked(invoke).mock.calls.find(([name]) => name === "start_download")?.[1] as {request: {id: string}}).request.id;
  }
  function savedQueue(status: "completed" | "paused", duplicate = false) {
    const first = {...createQueueItem("https://example.com/video", "first"), status: "completed", title: "Попередній файл"};
    const items = duplicate ? [first, createQueueItem(first.url, "second"), createQueueItem("https://example.com/third", "third")] : [first];
    localStorage.setItem(QUEUE_STORAGE_KEY, JSON.stringify({version: 1, status, items, activeItemId: null, settings: {outputDir: "/Downloads", mode: "video", quality: "best", audioFormat: "mp3", subtitles: false, multiItem: false, cookiesBrowser: null}}));
  }
  it("keeps output-less failure and its report after history reload", async () => {
    render(<App/>);
    const id = await startSingle();
    events.get("download-event")?.({payload: {id, kind: "failed", message: "Доступ відхилено", errorCode: "unknown"}});
    await waitFor(() => expect(JSON.parse(localStorage.getItem("downloadHistory.v1") || "[]")[0]?.diagnostics).toContain("Очищений"));
    cleanup(); render(<App/>);
    fireEvent.click(screen.getByRole("button", {name: /Історія/i}));
    await screen.findByText(/Доступ відхилено/);
    expect(screen.queryByRole("button", {name: "Показати"})).toBeNull();
    fireEvent.click(screen.getByRole("button", {name: "Подробиці"}));
    expect((screen.getByLabelText("Діагностика") as HTMLTextAreaElement).value).toContain("Очищений");
  });
  it("does not route single discovery into an old queue with the same URL", async () => {
    savedQueue("completed"); render(<App/>);
    const id = await startSingle();
    events.get("download-event")?.({payload: {id, kind: "failed", errorCode: "selection_required"}});
    events.get("media-candidates")?.({payload: {jobId: id, sessionId: "single-session", pageUrl: "https://example.com/video", urls: ["https://cdn.example.com/movie.mp4"]}});
    fireEvent.click(await screen.findByText(/cdn\.example\.com/));
    await waitFor(() => expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "start_download")).toHaveLength(2));
    const requests = vi.mocked(invoke).mock.calls.filter(([name]) => name === "start_download");
    expect(requests[1][1]).toMatchObject({request: {discoverySession: "single-session"}});
    expect(JSON.parse(localStorage.getItem(QUEUE_STORAGE_KEY) || "{}").items[0].status).toBe("completed");
  });
  it.each(["auth_required", "low_disk"])("pauses a queue on %s without starting the next item", async (errorCode) => {
    savedQueue("paused", true); render(<App/>);
    fireEvent.click(screen.getByRole("button", {name: /Пакетне завантаження/i}));
    fireEvent.click(await screen.findByRole("button", {name: /^Продовжити$/}));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_download", expect.anything()));
    const id = (vi.mocked(invoke).mock.calls.find(([name]) => name === "start_download")?.[1] as {request: {id: string}}).request.id;
    events.get("download-event")?.({payload: {id, kind: "failed", message: "Потрібне втручання", errorCode}});
    await waitFor(() => expect(JSON.parse(localStorage.getItem(QUEUE_STORAGE_KEY) || "{}").status).toBe("paused"));
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "start_download")).toHaveLength(1);
    expect(JSON.parse(localStorage.getItem(QUEUE_STORAGE_KEY) || "{}").items[1].status).toBe("interrupted");
  });
  it("recovers the failed duplicate row, not the first completed row", async () => {
    savedQueue("paused", true); render(<App/>);
    fireEvent.click(screen.getByRole("button", {name: /Пакетне завантаження/i}));
    fireEvent.click(await screen.findByRole("button", {name: /^Продовжити$/}));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_download", expect.anything()));
    const id = (vi.mocked(invoke).mock.calls.find(([name]) => name === "start_download")?.[1] as {request: {id: string}}).request.id;
    events.get("download-event")?.({payload: {id, kind: "failed", errorCode: "selection_required"}});
    events.get("media-candidates")?.({payload: {jobId: id, sessionId: "queue-session", pageUrl: "https://example.com/video", urls: ["https://cdn.example.com/movie.mp4"]}});
    fireEvent.click(await screen.findByText(/cdn\.example\.com/));
    await waitFor(() => {
      const queue = JSON.parse(localStorage.getItem(QUEUE_STORAGE_KEY) || "{}");
      expect(queue.items[0].status).toBe("completed");
      expect(queue.items[1].status).toBe("pending");
      expect(queue.status).toBe("paused");
    });
    const resumeButtons = screen.getAllByRole("button", {name: /^Продовжити$/});
    fireEvent.click(resumeButtons[resumeButtons.length - 1]);
    await waitFor(() => expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "start_download")).toHaveLength(2));
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "start_download")[1][1]).toMatchObject({request: {discoverySession: "queue-session", url: "https://example.com/video"}});
  });
  it("records completed playlist outputs even after the user cancels", async () => {
    render(<App/>);
    const id = await startSingle();
    fireEvent.click(screen.getByRole("button", {name: /^Зупинити$/}));
    fireEvent.click(await screen.findByRole("button", {name: "Так, скасувати"}));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("cancel_download", {id}));
    await screen.findByText(/Зупиняємо процес і завершуємо/);
    expect(screen.getByRole("progressbar", {name: "Завантаження відео"})).toBeTruthy();
    events.get("download-event")?.({payload: {id, kind: "cancelled", outputs: [{path: "/Downloads/готове.mp4", size: 400, title: "Готове", sourceUrl: "https://example.com/first"}]}});
    await waitFor(() => expect(JSON.parse(localStorage.getItem("downloadHistory.v1") || "[]")).toHaveLength(2));
    const entries = JSON.parse(localStorage.getItem("downloadHistory.v1") || "[]");
    expect(entries[0].path).toBe("/Downloads/готове.mp4");
    expect(entries[1].outcome).toBe("cancelled");
  });
  it("starts with one click without a remote URL precheck and shows terminal failure", async () => {
    render(<App/>);
    const input = screen.getByPlaceholderText(/https:\/\/youtube/);
    fireEvent.change(input, {target: {value: "https://example.com/video"}});
    expect(screen.queryByRole("button", {name: "Знайти відео в браузері"})).toBeNull();
    const start = screen.getByRole("button", {name: /^Завантажити$/});
    await waitFor(() => expect((start as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(start);
    await waitFor(() => expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "start_download")).toHaveLength(1));
    expect(vi.mocked(invoke).mock.calls.some(([name]) => name === "probe_url")).toBe(false);
    const request = vi.mocked(invoke).mock.calls.find(([name]) => name === "start_download")?.[1] as {request: {id: string}};
    events.get("download-event")?.({payload: {id: request.request.id, kind: "failed", message: "Контрольована помилка", percent: null, outputs: null}});
    await screen.findByText("Контрольована помилка");
    expect(screen.queryByText("Подробиці")).toBeNull();
    expect(screen.getByRole("button", {name: "Знайти відео в браузері"})).toBeTruthy();
    expect(screen.getByText(/На сайті натисніть/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", {name: "Закрити"}));
    await waitFor(() => expect(screen.queryByText("Контрольована помилка")).toBeNull());
  });
  it("keeps details out of the completed popup and saves them in history", async () => {
    render(<App/>);
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/youtube/), {target: {value: "https://example.com/video"}});
    const start = screen.getByRole("button", {name: /^Завантажити$/});
    await waitFor(() => expect((start as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(start);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_download", expect.anything()));
    const request = vi.mocked(invoke).mock.calls.find(([name]) => name === "start_download")?.[1] as {request: {id: string}};
    events.get("download-event")?.({payload: {id: request.request.id, kind: "completed", message: "Готово", percent: 100, outputs: [{path: "/Downloads/test.mp4", size: 1024, title: "Тестове відео", sourceUrl: "https://example.com/video"}]}});
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("download_diagnostics", {id: request.request.id}));
    expect(screen.queryByText("Подробиці")).toBeNull();
    fireEvent.click(screen.getByRole("button", {name: "Закрити"}));
    fireEvent.click(screen.getByRole("button", {name: /Історія/i}));
    fireEvent.click(await screen.findByRole("button", {name: "Подробиці"}));
    expect((screen.getByLabelText("Діагностика") as HTMLTextAreaElement).value).toBe("Очищений журнал тестового завантаження");
  });
  it("lets the user reject app closure while idle", async () => {
    render(<App/>);
    await waitFor(() => expect(events.has("app-close-requested")).toBe(true));
    events.get("app-close-requested")?.({payload: {activeDownloads: 0}});
    const stay = await screen.findByRole("button", {name: /Ні, залишитись/i});
    fireEvent.click(stay);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("dismiss_app_close_request"));
    expect(invoke).not.toHaveBeenCalledWith("confirm_app_close");
  });
});
