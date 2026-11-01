// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { canOfferBrowserDiscovery, DownloadTools, HistoryDetails, MediaCandidatePicker, RecoveryNotice } from "./MediaTools";
import { DialogSurface } from "./DialogSurface";

const mocks = vi.hoisted(() => ({ callback: null as null | ((event: {payload: unknown}) => void) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (event, callback) => { if (event === "media-candidates") mocks.callback = callback; return () => {}; }) }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe("interactive recovery and diagnostic controls", () => {
  it("does not offer media discovery for conversion, disk, login or network problems", () => {
    for (const code of ["low_disk", "auth_required", "conversion_failed", "network", "rate_limited", "start_failed", "photo_only"]) expect(canOfferBrowserDiscovery(code)).toBe(false);
    expect(canOfferBrowserDiscovery("unsupported")).toBe(true);
    render(<DownloadTools url="https://example.com/video" jobId="job"/>);
    expect(screen.queryByRole("button")).toBeNull();
  });
  it("shows saved diagnostics only after opening history details", async () => {
    render(<HistoryDetails title="Тестове відео" diagnostics="Очищений збережений журнал"/>);
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByText("Подробиці"));
    expect((screen.getByLabelText("Діагностика") as HTMLTextAreaElement).value).toBe("Очищений збережений журнал");
    expect(invoke).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("Закрити подробиці"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
  it("handles older history without a saved diagnostic report", () => {
    render(<HistoryDetails title="Давнє відео"/>);
    fireEvent.click(screen.getByText("Подробиці"));
    expect(screen.getByText(/Журнал цього завантаження не збережено/)).toBeTruthy();
    expect(screen.queryByText("Скопіювати діагностику")).toBeNull();
  });
  it("doesn't remove interrupted work until the explicit button is pressed", async () => {
    vi.mocked(invoke).mockImplementation(async (command) => command === "interrupted_work" ? [{id: "owned", directory: "/downloads/.yt-dlp-bd-owned.tmp"}] : undefined as never);
    render(<RecoveryNotice ready/>);
    await screen.findByText("Видалити залишки");
    expect(invoke).not.toHaveBeenCalledWith("remove_interrupted_work", expect.anything());
    fireEvent.click(screen.getByText("Видалити залишки"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("remove_interrupted_work", { id: "owned" }));
  });
  it("opens browser discovery only after an explicit click", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    render(<DownloadTools url="https://example.com/video" jobId="job" canDiscover/>);
    expect(screen.queryByText("Подробиці")).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("Знайти відео в браузері"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("open_media_browser", {pageUrl: "https://example.com/video", jobId: "job"}));
  });
  it("asks for a source and never exposes its signed query in the label", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    const selected = vi.fn();
    render(<MediaCandidatePicker busy={false} onSelected={selected}/>);
    mocks.callback?.({payload: {sessionId: "session", jobId: "job", pageUrl: "https://example.com/post", urls: ["https://cdn.example.com/movie.mp4?token=SECRET"]}});
    const button = await screen.findByText(/cdn\.example\.com/);
    expect(button.textContent).not.toContain("SECRET");
    expect(selected).not.toHaveBeenCalled();
    fireEvent.click(button);
    await waitFor(() => expect(selected).toHaveBeenCalledWith(expect.objectContaining({sessionId: "session", jobId: "job", pageUrl: "https://example.com/post"})));
  });
  it("keeps focus inside a dialog and allows safe Escape", async () => {
    const close = vi.fn();
    render(<DialogSurface className="test" labelledBy="title" onEscape={close}><h2 id="title">Test</h2><button>First</button><button>Last</button></DialogSurface>);
    screen.getByText("Last").focus();
    fireEvent.keyDown(screen.getByRole("dialog"), {key: "Tab"});
    expect(document.activeElement).toBe(screen.getByText("First"));
    fireEvent.keyDown(screen.getByRole("dialog"), {key: "Escape"});
    expect(close).toHaveBeenCalledOnce();
  });
});
