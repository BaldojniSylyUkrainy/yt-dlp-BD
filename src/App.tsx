import { useCallback, useEffect, useRef, useState, type ClipboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { downloadDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import brandLogo from "./assets/logo-call-me-hands.png";
import packageMetadata from "../package.json";
import { maintenanceDue, maintenanceRetryAt, friendlyMaintenanceError } from "./maintenance";
import { DialogSurface } from "./DialogSurface";
import { canOfferBrowserDiscovery, DownloadTools, MediaCandidatePicker, RecoveryNotice } from "./MediaTools";
import { formatByteSize } from "./format";
import { applyHistoryFileStatuses, applyHistoryThumbnailCache, fileTitle, HISTORY_LIMIT, historyPaths, limitHistoryDiagnostics, loadHistory, parentDirectory, shouldDeleteUnusedHistoryThumbnail, type HistoryDownloadSettings, type HistoryEntry, type HistoryFileStatus } from "./history";
import { HistoryView } from "./HistoryView";
import { Icon } from "./Icon";
import { QueueProgressModal, QueueView } from "./QueueViews";
import { RuntimePreparationOverlay, RuntimeRow, StartupOverlay, type RuntimeInstallProgress, type RuntimeStage, type RuntimeStatus } from "./RuntimeViews";
import {
  commitQueueInput,
  nextPendingQueueItem,
  normalizeCookieBrowser,
  normalizeHttpUrl,
  parseQueueStorage,
  QUEUE_LIMIT,
  QUEUE_STORAGE_KEY,
  queueHasActiveProcess,
  queuePreventsOtherWork,
  queueProgress,
  resetEntireQueueForReplay,
  resetQueueItemsForRetry,
  type DownloadQueue,
  type QueueItem,
  type QueueSettings,
} from "./queue";
import "./App.css";

const APP_RELEASE_VERSION = packageMetadata.releaseVersion;
const HISTORY_STORAGE_KEY = "downloadHistory.v1";
export const UPDATE_CHECK_DELAYS = [0, 5_000, 30_000] as const;
const UPDATE_CHECK_INTERVAL = 6 * 60 * 60 * 1_000;

export function shouldPlayCompletionSound(kind: DownloadEvent["kind"]): boolean {
  return kind === "completed";
}

export function shouldPlayQueueCompletionSound(items: QueueItem[]): boolean {
  return items.some((item) => item.status === "completed");
}

type VpnStatus = {
  detected: boolean;
  interfaceName: string | null;
  confidence: string;
  detail: string;
};

export type DownloadEvent = {
  id: string;
  kind: "metadata" | "progress" | "postprocess" | "conversion_progress" | "storage_estimate" | "retrying" | "log" | "completed" | "failed" | "cancelled" | "auth_required";
  percent: number | null;
  speed: string | null;
  eta: string | null;
  message: string | null;
  storage: StorageEstimate | null;
  outputs: DownloadOutput[] | null;
  title?: string | null;
  thumbnail?: string | null;
  uploader?: string | null;
  extractor?: string | null;
  errorCode?: string | null;
};

type DownloadOutput = {
  path: string;
  size: number;
  sourceUrl?: string | null;
  title?: string | null;
  thumbnail?: string | null;
  uploader?: string | null;
  extractor?: string | null;
};

export function isBlockingQueueFailure(kind: string, code?: string | null): boolean {
  return kind === "auth_required" || ["low_disk", "rate_limited", "selection_required", "auth_required", "destination_unavailable"].includes(code || "");
}

type HistoryContext = {
  sourceUrl: string;
  title: string;
  thumbnail: string | null;
  uploader: string | null;
  extractor: string | null;
  settings: HistoryDownloadSettings;
};

type StorageEstimate = {
  estimatedSize: number;
  requiredSpace: number;
  availableSpace: number | null;
  sufficient: boolean | null;
};

type PreflightResult = {
  availableSpace: number;
  sufficient: boolean;
};

export function preflightAllowsStart(result: PreflightResult): boolean {
  return result.sufficient;
}

export function runtimeInstallCommand(stage: Exclude<RuntimeStage, null>): string {
  if (stage === "ytDlp") return "update_ytdlp";
  if (stage === "ffmpeg") return "install_ffmpeg";
  return "install_deno";
}

export function runtimeUpdateKey(stage: Exclude<RuntimeStage, null>): string {
  if (stage === "ytDlp") return "runtimeUpdate.ytDlpNightly.v2";
  return `runtimeUpdate.${stage}`;
}

export function defaultCookieBrowser(platform: string, stored: string | null): string {
  const normalized = normalizeCookieBrowser(stored);
  if (normalized) return normalized;
  return platform === "windows" ? "edge" : "chrome";
}

export type Job = {
  errorCode?: string | null;
  id: string;
  url: string;
  title: string;
  status: "starting" | "downloading" | "postprocessing" | "converting" | "completed" | "failed" | "cancelled" | "auth_required";
  percent: number;
  speed: string;
  eta: string;
  message: string;
  storage: StorageEstimate | null;
  playlist: boolean;
  outputFormat: string;
  outputPaths?: string[];
};

type AppCloseRequest = {
  activeDownloads: number;
};

export type AppCloseActivity = {
  singleStage: Job["status"] | null;
  batchDownload: boolean;
  runtimeMaintenance: boolean;
  appUpdate: boolean;
  linkCheck: boolean;
  backendDownloads: number;
};

export function appCloseActivityLabels(activity: AppCloseActivity): string[] {
  const labels: string[] = [];
  if (activity.singleStage === "converting") labels.push("Конвертація файла ще триває.");
  else if (activity.singleStage && ["starting", "downloading", "postprocessing"].includes(activity.singleStage)) labels.push("Завантаження або обробка файла ще триває.");
  if (activity.batchDownload) labels.push("Пакетне завантаження ще виконується.");
  if (activity.runtimeMaintenance) labels.push("Компоненти зараз перевіряються або оновлюються.");
  if (activity.appUpdate) labels.push("Оновлення застосунку ще завантажується або встановлюється.");
  if (activity.linkCheck) labels.push("Посилання ще перевіряється або готується до завантаження.");
  if (activity.backendDownloads > 0 && !activity.singleStage && !activity.batchDownload) labels.push("Фонове завантаження або конвертація ще триває.");
  return labels;
}

export function applyDownloadEvent(current: Job, payload: DownloadEvent): Job | null {
  if (payload.kind === "cancelled") return null;
  if (payload.kind === "metadata") {
    return { ...current, title: payload.title || current.title };
  }
  if (payload.kind === "storage_estimate") {
    return { ...current, storage: payload.storage || current.storage };
  }
  if (payload.kind === "retrying") {
    return {
      ...current,
      status: "starting",
      speed: "—",
      eta: "—",
      message: payload.message || "Відновлюємо завантаження…",
    };
  }
  if (payload.kind === "progress") {
    return {
      ...current,
      status: "downloading",
      percent: payload.percent ?? current.percent,
      speed: payload.speed ?? current.speed,
      eta: payload.eta ?? current.eta,
      title: payload.title || current.title,
      message: payload.message || (current.status === "starting" ? "Завантажуємо…" : current.message),
    };
  }
  if (payload.kind === "postprocess") {
    return {
      ...current,
      status: "postprocessing",
      speed: payload.speed || "yt-dlp",
      eta: "—",
      message: payload.message || "Об’єднуємо завантажені потоки… Не закривайте застосунок",
    };
  }
  if (payload.kind === "conversion_progress") {
    return {
      ...current,
      status: "converting",
      percent: payload.percent ?? current.percent,
      speed: payload.speed || "FFmpeg",
      eta: payload.eta || "—",
      message: payload.message || "Створюємо сумісний MP4… Не закривайте застосунок",
    };
  }
  if (payload.kind === "log") {
    return { ...current, message: payload.message || current.message };
  }
  return {
    ...current,
    status: payload.kind,
    errorCode: payload.errorCode,
    percent: payload.kind === "completed" ? 100 : current.percent,
    outputPaths: payload.outputs?.map((output) => output.path) ?? current.outputPaths,
    message: payload.kind === "auth_required"
      ? "Потрібен вхід через браузер"
      : payload.kind === "completed" ? payload.message || "Готово" : payload.message || current.message,
  };
}

function hostFromInput(value: string): string | null {
  try {
    return new URL(/^https?:\/\//i.test(value.trim()) ? value.trim() : `https://${value.trim()}`).hostname.toLowerCase();
  } catch {
    return null;
  }
}

function normalizeInputUrl(value: string): string | null {
  return normalizeHttpUrl(value);
}

export function inputUrlState(value: string): "idle" | "valid" | "invalid" {
  if (!value.trim()) return "idle";
  return normalizeInputUrl(value) ? "valid" : "invalid";
}

function isRussianDomain(host: string): boolean {
  return host === "ru" || host.endsWith(".ru") || host === "xn--p1ai" || host.endsWith(".xn--p1ai");
}

export function isLikelyMultiItemUrl(value: string): boolean {
  try {
    const parsed = new URL(/^https?:\/\//i.test(value.trim()) ? value.trim() : `https://${value.trim()}`);
    const host = parsed.hostname.toLowerCase();
    const path = parsed.pathname.toLowerCase();
    if (host === "youtu.be" || host.endsWith(".youtu.be") || host === "youtube.com" || host.endsWith(".youtube.com")) {
      return path === "/playlist" || parsed.searchParams.has("list");
    }
    if (host === "threads.com" || host.endsWith(".threads.com") || host === "threads.net" || host.endsWith(".threads.net")) {
      return /^\/(?:share\/[^/]+|@[^/]+\/(?:post|t)\/[^/]+)\/?$/i.test(parsed.pathname);
    }
    return ["/playlist/", "/playlists/", "/sets/", "/album/", "/albums/", "/collection/", "/collections/", "/showcase/"].some((segment) => path.includes(segment));
  } catch {
    return false;
  }
}

/** The address names only a collection: yt-dlp's `--no-playlist` has no single item to pick. */
export function isWholeCollectionUrl(value: string): boolean {
  if (!isLikelyMultiItemUrl(value)) return false;
  try {
    const parsed = new URL(/^https?:\/\//i.test(value.trim()) ? value.trim() : `https://${value.trim()}`);
    const host = parsed.hostname.toLowerCase();
    if (host === "youtu.be" || host.endsWith(".youtu.be")) return false;
    if (host === "youtube.com" || host.endsWith(".youtube.com")) return !parsed.searchParams.has("v");
    return !(host === "threads.com" || host.endsWith(".threads.com") || host === "threads.net" || host.endsWith(".threads.net"));
  } catch {
    return false;
  }
}

export function shouldShowMultiItemIntent(value: string, itemCount?: number | null): boolean {
  return isLikelyMultiItemUrl(value) || Boolean(itemCount && itemCount > 1);
}

export function playlistIntentToMultiItem(intent: "single" | "playlist"): boolean {
  return intent === "playlist";
}

export function DownloadCollectionIntent({ url, intent, onIntentChange }: {
  url: string;
  intent: "single" | "playlist";
  onIntentChange: (intent: "single" | "playlist") => void;
}) {
  if (!shouldShowMultiItemIntent(url)) return null;
  if (isWholeCollectionUrl(url)) return <div className="collection-intent">
    <span className="field-label">Це ціла добірка</span>
    <small>Завантажаться всі доступні елементи з безпечними паузами між ними.</small>
  </div>;
  return <div className="collection-intent">
    <span className="field-label">Що взяти з добірки</span>
    <div className="segmented">
      <button type="button" className={intent === "single" ? "selected" : ""} onClick={() => onIntentChange("single")}>Лише це відео</button>
      <button type="button" className={intent === "playlist" ? "selected" : ""} onClick={() => onIntentChange("playlist")}>Уся добірка</button>
    </div>
    <small>{intent === "single" ? "За замовчуванням завантажиться лише поточний ролик." : "Завантажаться всі доступні елементи з безпечними паузами між ними."}</small>
  </div>;
}

function jobStageNumber(status: Job["status"]): string | null {
  if (status === "starting") return "0";
  if (status === "downloading") return "1";
  if (status === "postprocessing" || status === "converting") return "2";
  return null;
}

function youtubeThumbnailFromInput(value: string): string | null {
  try {
    const parsed = new URL(/^https?:\/\//i.test(value.trim()) ? value.trim() : `https://${value.trim()}`);
    const host = parsed.hostname.toLowerCase();
    let id: string | null = null;
    if (host === "youtu.be" || host.endsWith(".youtu.be")) {
      id = parsed.pathname.split("/").filter(Boolean)[0] || null;
    } else if (host === "youtube.com" || host.endsWith(".youtube.com")) {
      id = parsed.searchParams.get("v");
      if (!id) {
        const parts = parsed.pathname.split("/").filter(Boolean);
        if (["shorts", "embed", "live"].includes(parts[0])) id = parts[1] || null;
      }
    }
    return id && /^[A-Za-z0-9_-]{6,}$/.test(id) ? `https://i.ytimg.com/vi/${id}/hqdefault.jpg` : null;
  } catch {
    return null;
  }
}

function App() {
  const maintenanceStarted = useRef(false);
  const maintenanceChecking = useRef(false);
  const appMounted = useRef(true);
  const maintenanceFollowUp = useRef<number | null>(null);
  const startupFade = useRef<number | null>(null);
  const approvedVpnUrls = useRef(new Set<string>());
  const cancelledDownloadRestore = useRef<{ url: string; playlist: boolean } | null>(null);
  const mainContentRef = useRef<HTMLElement>(null);
  const jobTitleTextRef = useRef<HTMLDivElement>(null);
  const pendingDownloadEvents = useRef(new Map<string, DownloadEvent[]>());
  const historyContexts = useRef(new Map<string, HistoryContext>());
  const discoverySelections = useRef(new Map<string, { sessionId: string; pageUrl: string }>());
  const [runtime, setRuntime] = useState<RuntimeStatus | null>(null);
  const [runtimeBusy, setRuntimeBusy] = useState(false);
  const [runtimeStage, setRuntimeStage] = useState<RuntimeStage>(null);
  const [runtimeError, setRuntimeError] = useState("");
  const [runtimeFailures, setRuntimeFailures] = useState(new Set<Exclude<RuntimeStage, null>>());
  const [runtimeInstallProgress, setRuntimeInstallProgress] = useState<RuntimeInstallProgress | null>(null);
  const [runtimePending, setRuntimePending] = useState<Exclude<RuntimeStage, null>[]>([]);
  const [url, setUrl] = useState("");
  const urlInputRef = useRef<HTMLInputElement>(null);
  const [activeView, setActiveView] = useState<"download" | "queue" | "history">("download");
  const [historyEntries, setHistoryEntries] = useState<HistoryEntry[]>([]);
  const historyEntriesRef = useRef(historyEntries);
  const [historyHydrated, setHistoryHydrated] = useState(false);
  const [historyRetryPending, setHistoryRetryPending] = useState<string | null>(null);
  const historyPreparationInFlight = useRef(false);
  const [outputDir, setOutputDir] = useState("");
  const [outputFreeSpace, setOutputFreeSpace] = useState<number | null>(null);
  const [unavailableOutputDir, setUnavailableOutputDir] = useState<string | null>(null);
  const [mode, setMode] = useState<"video" | "audio">("video");
  const [quality, setQuality] = useState("best");
  const [audioFormat, setAudioFormat] = useState("mp3");
  const [subtitles, setSubtitles] = useState(false);
  const [playlistIntent, setPlaylistIntent] = useState<"single" | "playlist">("single");
  const [job, setJob] = useState<Job | null>(null);
  const [formError, setFormError] = useState("");
  const [vpnWarning, setVpnWarning] = useState<{ host: string; url: string; status: VpnStatus; queueItemId: string | null } | null>(null);
  const [pendingStart, setPendingStart] = useState(false);
  const [update, setUpdate] = useState<Update | null>(null);
  const updateAvailableRef = useRef(false);
  const updateCheckInFlight = useRef<Promise<"found" | "none" | "error"> | null>(null);
  const [updatePromptOpen, setUpdatePromptOpen] = useState(false);
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateCheckError, setUpdateCheckError] = useState("");
  const [updateInstallError, setUpdateInstallError] = useState("");
  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateProgress, setUpdateProgress] = useState(0);
  const initialPlatform = navigator.userAgent.includes("Windows") ? "windows" : "macos";
  const [cookieBrowser, setCookieBrowser] = useState(defaultCookieBrowser(initialPlatform, null));
  const [startupBusy, setStartupBusy] = useState(true);
  const [startupProgress, setStartupProgress] = useState(10);
  const [startupMessage, setStartupMessage] = useState("Відкриваємо застосунок…");
  const [mainScrollable, setMainScrollable] = useState(false);
  const [jobValueFontSize, setJobValueFontSize] = useState(24);
  const [cancelConfirmOpen, setCancelConfirmOpen] = useState(false);
  const [cancelBusy, setCancelBusy] = useState(false);
  const [cancelError, setCancelError] = useState("");
  const [downloadQueue, setDownloadQueue] = useState<DownloadQueue | null>(null);
  const downloadQueueRef = useRef<DownloadQueue | null>(null);
  const [queueHydrated, setQueueHydrated] = useState(false);
  const [queueInput, setQueueInput] = useState("");
  const [queueNotice, setQueueNotice] = useState("");
  const [queueAlert, setQueueAlert] = useState<{ title: string; message: string } | null>(null);
  const [queueMonitorOpen, setQueueMonitorOpen] = useState(false);
  const [closeRequest, setCloseRequest] = useState<AppCloseRequest | null>(null);
  const [closeBusy, setCloseBusy] = useState(false);
  const [closeError, setCloseError] = useState("");
  const queueLaunchInFlight = useRef(false);
  const queueCancellationIntent = useRef<"skip" | "stop" | null>(null);
  const sleepPreventionChain = useRef<Promise<unknown>>(Promise.resolve());

  const active = job && ["starting", "downloading", "postprocessing", "converting"].includes(job.status);
  const queueProcessActive = queueHasActiveProcess(downloadQueue);
  const queueBusy = queuePreventsOtherWork(downloadQueue);
  const anyProcessActive = Boolean(active || queueProcessActive);
  const workBusyRef = useRef(false);
  workBusyRef.current = anyProcessActive || queueBusy || pendingStart || updateBusy;
  const runtimeReady = Boolean(runtime?.ytDlp.installed && runtime?.ffmpeg.installed && runtime?.deno.installed);
  const isWindows = runtime?.platform === "windows";
  const normalizedInputUrl = normalizeInputUrl(url);
  const urlState = inputUrlState(url);
  const closeActivities = appCloseActivityLabels({
    singleStage: active ? job?.status || null : null,
    batchDownload: Boolean(queueProcessActive || downloadQueue?.status === "running"),
    runtimeMaintenance: runtimeBusy,
    appUpdate: updateBusy,
    linkCheck: pendingStart,
    backendDownloads: closeRequest?.activeDownloads || 0,
  });

  const updateQueue = useCallback((producer: (current: DownloadQueue | null) => DownloadQueue | null) => {
    const next = producer(downloadQueueRef.current);
    downloadQueueRef.current = next;
    setDownloadQueue(next);
    return next;
  }, []);

  useEffect(() => {
    const enabled = Boolean(active || downloadQueue?.status === "running" || downloadQueue?.activeItemId);
    sleepPreventionChain.current = sleepPreventionChain.current
      .catch(() => undefined)
      .then(() => invoke("set_queue_sleep_prevention", { enabled }))
      .catch(() => undefined);
  }, [active, downloadQueue?.activeItemId, downloadQueue?.status]);

  useEffect(() => {
    let disposed = false;
    const showCloseConfirmation = (request: AppCloseRequest) => {
      if (disposed) return;
      setCloseError("");
      setCloseBusy(false);
      setCloseRequest(request);
    };
    const unlisten = listen<AppCloseRequest>("app-close-requested", ({ payload }) => showCloseConfirmation(payload));
    void invoke<AppCloseRequest | null>("take_app_close_request")
      .then((request) => { if (request) showCloseConfirmation(request); })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten.then((dispose) => dispose());
    };
  }, []);

  useEffect(() => {
    let timer = 0;
    const frame = window.requestAnimationFrame(() => {
      timer = window.setTimeout(() => {
        const loadedHistory = loadHistory();
        historyEntriesRef.current = loadedHistory;
        setHistoryEntries(loadedHistory);
        setHistoryHydrated(true);
        try {
          const savedOutputDir = localStorage.getItem("outputDir");
          if (savedOutputDir) {
            setOutputDir(savedOutputDir);
          } else {
            void downloadDir().then((directory) => {
              setOutputDir(directory);
              localStorage.setItem("outputDir", directory);
            }).catch(() => undefined);
          }
          const fallbackSettings: QueueSettings = {
            outputDir: savedOutputDir || "",
            mode: "video",
            quality: "best",
            audioFormat: "mp3",
            subtitles: false,
            multiItem: false,
            cookiesBrowser: null,
          };
          const storedQueue = parseQueueStorage(localStorage.getItem(QUEUE_STORAGE_KEY), fallbackSettings);
          const restoredQueue = storedQueue?.settings.cookiesBrowser === "safari" ? {
            ...storedQueue,
            settings: { ...storedQueue.settings, cookiesBrowser: null },
          } : storedQueue;
          downloadQueueRef.current = restoredQueue;
          setDownloadQueue(restoredQueue);
          setQueueHydrated(true);
          setCookieBrowser(defaultCookieBrowser(initialPlatform, localStorage.getItem("cookieBrowser")));
        } catch {
          void downloadDir().then(setOutputDir).catch(() => undefined);
          setQueueHydrated(true);
        }
      }, 0);
    });
    return () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timer);
    };
  }, [initialPlatform]);

  useEffect(() => {
    if (!queueHydrated) return;
    const save = () => { try {
      if (downloadQueue) localStorage.setItem(QUEUE_STORAGE_KEY, JSON.stringify(downloadQueue));
      else localStorage.removeItem(QUEUE_STORAGE_KEY);
    } catch {
      // A queue can continue in memory even when local persistence is unavailable.
    } };
    const timer = window.setTimeout(save, 750);
    window.addEventListener("pagehide", save);
    return () => { window.clearTimeout(timer); window.removeEventListener("pagehide", save); };
  }, [downloadQueue, queueHydrated]);

  const queueCheckpoint = downloadQueue ? JSON.stringify([downloadQueue.status, downloadQueue.activeItemId, downloadQueue.settings, downloadQueue.items.map(({ id, status, url }) => [id, status, url])]) : "";
  useEffect(() => {
    if (!queueHydrated) return;
    try {
      const current = downloadQueueRef.current;
      if (current) localStorage.setItem(QUEUE_STORAGE_KEY, JSON.stringify(current));
      else localStorage.removeItem(QUEUE_STORAGE_KEY);
    } catch { /* Continue safely in memory if storage is full. */ }
  }, [queueCheckpoint, queueHydrated]);

  useEffect(() => {
    if (activeView !== "queue" || downloadQueue) return;
    const now = new Date().toISOString();
    updateQueue(() => ({
      version: 1,
      status: "draft",
      settings: currentQueueSettings(),
      items: [],
      activeItemId: null,
      createdAt: now,
      updatedAt: now,
    }));
  }, [activeView, downloadQueue, updateQueue]);

  useEffect(() => {
    if (!outputDir || !downloadQueue || downloadQueue.settings.outputDir) return;
    updateQueue((current) => current ? {
      ...current,
      settings: { ...current.settings, outputDir },
      updatedAt: new Date().toISOString(),
    } : current);
  }, [downloadQueue, outputDir, updateQueue]);

  useEffect(() => {
    if (cookieBrowser !== "safari") return;
    const fallback = isWindows ? "edge" : "chrome";
    setCookieBrowser(fallback);
    localStorage.setItem("cookieBrowser", fallback);
  }, [cookieBrowser, isWindows]);

  useEffect(() => {
    const restored = cancelledDownloadRestore.current;
    if (restored?.url === url) {
      setPlaylistIntent(restored.playlist ? "playlist" : "single");
      cancelledDownloadRestore.current = null;
    } else {
      setPlaylistIntent("single");
    }
  }, [url]);

  useEffect(() => {
    if (active) return;
    setCancelConfirmOpen(false);
    setCancelBusy(false);
    setCancelError("");
  }, [active]);

  useEffect(() => {
    const text = jobTitleTextRef.current;
    if (!text) return;
    const updateSize = () => {
      const measuredHeight = Math.ceil(text.getBoundingClientRect().height);
      setJobValueFontSize(Math.max(24, Math.min(68, measuredHeight)));
    };
    const resizeObserver = new ResizeObserver(updateSize);
    resizeObserver.observe(text);
    updateSize();
    return () => resizeObserver.disconnect();
  }, [job?.id]);

  useEffect(() => {
    const mainContent = mainContentRef.current;
    if (!mainContent) return;

    let animationFrame = 0;
    const updateScrollMode = () => {
      window.cancelAnimationFrame(animationFrame);
      animationFrame = window.requestAnimationFrame(() => {
        const overflow = mainContent.scrollHeight - mainContent.clientHeight;
        const shouldScroll = overflow > 56;
        setMainScrollable((current) => current === shouldScroll ? current : shouldScroll);
        if (!shouldScroll && mainContent.scrollTop !== 0) mainContent.scrollTop = 0;
      });
    };

    const resizeObserver = new ResizeObserver(updateScrollMode);
    resizeObserver.observe(mainContent);
    Array.from(mainContent.children).forEach((child) => resizeObserver.observe(child));
    window.addEventListener("resize", updateScrollMode);
    updateScrollMode();

    return () => {
      window.cancelAnimationFrame(animationFrame);
      resizeObserver.disconnect();
      window.removeEventListener("resize", updateScrollMode);
    };
  }, [activeView]);

  useEffect(() => {
    historyEntriesRef.current = historyEntries;
    if (!historyHydrated) return;
    try {
      localStorage.setItem(HISTORY_STORAGE_KEY, JSON.stringify(historyEntries.slice(0, HISTORY_LIMIT)));
    } catch {
      // History is helpful but must never interrupt downloading when storage is unavailable.
    }
  }, [historyEntries, historyHydrated]);

  useEffect(() => {
    if (activeView !== "history" || historyEntries.length === 0) return;
    let disposed = false;
    let checking = false;
    const refreshAvailability = async () => {
      if (checking) return;
      checking = true;
      try {
        const paths = historyPaths(historyEntriesRef.current);
        if (paths.length === 0) return;
        const statuses = await invoke<HistoryFileStatus[]>("inspect_history_files", { paths });
        if (disposed) return;
        setHistoryEntries((current) => applyHistoryFileStatuses(current, statuses));
      } catch {
        // Keep the last known state if a removable volume is temporarily unavailable.
      } finally {
        checking = false;
      }
    };
    refreshAvailability();
    const timer = window.setInterval(refreshAvailability, 10_000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [activeView, historyEntries.length]);

  const refreshManagedComponents = useCallback(async () => {
    if (!appMounted.current || maintenanceChecking.current) return;
    const now = Date.now();
    const schedules = [
      { command: "update_ytdlp", stage: "ytDlp" as const, key: runtimeUpdateKey("ytDlp"), interval: 24 * 60 * 60 * 1000 },
      { command: "install_ffmpeg", stage: "ffmpeg" as const, key: runtimeUpdateKey("ffmpeg"), interval: 7 * 24 * 60 * 60 * 1000 },
      { command: "install_deno", stage: "deno" as const, key: runtimeUpdateKey("deno"), interval: 7 * 24 * 60 * 60 * 1000 },
    ];
    const due = schedules.filter(({ key, interval }) => maintenanceDue(localStorage, key, interval, now));
    if (!due.length) return;
    maintenanceChecking.current = true;
    setRuntimeError("");
    const issues: string[] = [];
    setRuntimePending(due.map(({ stage }) => stage));
    try {
      for (const component of due) {
        setRuntimeStage(component.stage);
        setRuntimePending((current) => current.filter((stage) => stage !== component.stage));
        setRuntimeInstallProgress(null);
        try {
          localStorage.setItem(`${component.key}.attempt`, String(now));
          const needsUpdate = await invoke<boolean>("runtime_update_available", { component: component.stage });
          if (needsUpdate) {
            if (workBusyRef.current) {
              localStorage.removeItem(`${component.key}.attempt`);
              continue; // Installation is deferred until the next idle check.
            }
            // Stays busy until the whole run ends so the overlay does not flicker
            // between components; the finally block below clears it.
            setRuntimeBusy(true);
            await invoke<void>(component.command);
          }
          localStorage.setItem(component.key, String(now));
          localStorage.removeItem(`${component.key}.retryAt`);
          setRuntimeFailures((current) => {
            const next = new Set(current);
            next.delete(component.stage);
            return next;
          });
        } catch (error) {
          localStorage.setItem(`${component.key}.retryAt`, String(maintenanceRetryAt(error, now)));
          issues.push(friendlyMaintenanceError(error));
          setRuntimeFailures((current) => new Set(current).add(component.stage));
        }
      }
      setRuntime(await invoke<RuntimeStatus>("runtime_status"));
      setRuntimeError(issues.join(" · "));
    } finally {
      maintenanceChecking.current = false;
      setRuntimeStage(null);
      setRuntimePending([]);
      setRuntimeInstallProgress(null);
      setRuntimeBusy(false);
    }
  }, []);

  useEffect(() => {
    if (startupBusy || !runtimeReady) return;
    const timer = window.setInterval(() => { if (!workBusyRef.current) void refreshManagedComponents(); }, 60_000);
    return () => window.clearInterval(timer);
  }, [startupBusy, runtimeReady, refreshManagedComponents]);

  const maintainRuntime = useCallback(async () => {
    setStartupBusy(true);
    setStartupProgress(18);
    setStartupMessage("Перевіряємо локальні компоненти…");
    setRuntimeError("");
    const issues: string[] = [];
    try {
      const status = await invoke<RuntimeStatus>("runtime_status");
      if (!appMounted.current) return;
      setRuntime(status);
      setStartupProgress(62);
      const missing = [
        { stage: "ytDlp" as const, label: "yt-dlp", installed: status.ytDlp.installed, command: "install_ytdlp" },
        { stage: "ffmpeg" as const, label: "ffmpeg", installed: status.ffmpeg.installed, command: "install_ffmpeg" },
        { stage: "deno" as const, label: "Deno", installed: status.deno.installed, command: "install_deno" },
      ].filter(({ installed }) => !installed);
      if (!missing.length) {
        setRuntimeBusy(false);
        maintenanceFollowUp.current = window.setTimeout(() => { void refreshManagedComponents(); }, 800);
        return;
      }
      setRuntimeBusy(true);
      setRuntimePending(missing.map(({ stage }) => stage));
      for (const [index, component] of missing.entries()) {
        setRuntimeStage(component.stage);
        setRuntimePending((current) => current.filter((stage) => stage !== component.stage));
        setStartupMessage(`Встановлюємо ${component.label}…`);
        setStartupProgress(62 + Math.round((index / missing.length) * 30));
        try {
          await invoke<void>(component.command);
          localStorage.setItem(runtimeUpdateKey(component.stage), String(Date.now()));
          setRuntimeFailures((current) => {
            const next = new Set(current);
            next.delete(component.stage);
            return next;
          });
        } catch (error) {
          issues.push(`${component.label}: ${String(error)}`);
          setRuntimeFailures((current) => new Set(current).add(component.stage));
        }
      }
      setRuntime(await invoke<RuntimeStatus>("runtime_status"));
      setRuntimeError(issues.join(" · "));
    } catch (error) {
      setRuntimeError(String(error));
    } finally {
      setRuntimeStage(null);
      setRuntimePending([]);
      setRuntimeBusy(false);
      setStartupProgress(100);
      setStartupMessage("Готово");
      if (appMounted.current) startupFade.current = window.setTimeout(() => setStartupBusy(false), 320);
    }
  }, [refreshManagedComponents]);

  async function retryRuntimeComponent(stage: Exclude<RuntimeStage, null>) {
    if (maintenanceChecking.current || workBusyRef.current) return;
    const retryAt = Number(localStorage.getItem(`${runtimeUpdateKey(stage)}.retryAt`) || 0);
    if (Date.now() < retryAt) {
      setRuntimeError(`Наступна перевірка можлива після ${new Date(retryAt).toLocaleTimeString("uk-UA")}. Встановлені компоненти залишаються доступними.`);
      return;
    }
    const command = runtimeInstallCommand(stage);
    maintenanceChecking.current = true;
    const maintenanceKey = runtimeUpdateKey(stage);
    localStorage.setItem(`${maintenanceKey}.attempt`, String(Date.now()));
    setRuntimeBusy(true);
    setRuntimeStage(stage);
    setRuntimeError("");
    setRuntimeInstallProgress(null);
    try {
      await invoke<void>(command);
      setRuntime(await invoke<RuntimeStatus>("runtime_status"));
      setRuntimeFailures((current) => {
        const next = new Set(current);
        next.delete(stage);
        return next;
      });
      localStorage.setItem(runtimeUpdateKey(stage), String(Date.now()));
      localStorage.removeItem(`${maintenanceKey}.retryAt`);
    } catch (error) {
      localStorage.setItem(`${maintenanceKey}.retryAt`, String(maintenanceRetryAt(error, Date.now())));
      setRuntimeError(friendlyMaintenanceError(error));
      setRuntimeFailures((current) => new Set(current).add(stage));
    } finally {
      maintenanceChecking.current = false;
      setRuntimeStage(null);
      setRuntimeBusy(false);
      setRuntimeInstallProgress(null);
    }
  }

  useEffect(() => {
    const unlisten = listen<RuntimeInstallProgress>("runtime-install-progress", ({ payload }) => {
      setRuntimeInstallProgress(payload);
    });
    return () => { unlisten.then((dispose) => dispose()); };
  }, []);

  const checkForAppUpdate = useCallback((surfaceError = false) => {
    if (updateAvailableRef.current) return Promise.resolve<"found">("found");
    if (updateCheckInFlight.current) return updateCheckInFlight.current;
    setUpdateChecking(true);
    const request = check({
      timeout: 20_000,
      headers: { "Cache-Control": "no-cache", Pragma: "no-cache" },
    }).then((candidate) => {
      if (!candidate) {
        if (surfaceError) setUpdateCheckError("");
        return "none" as const;
      }
      updateAvailableRef.current = true;
      setUpdate(candidate);
      setUpdateCheckError("");
      setUpdateInstallError("");
      setUpdatePromptOpen(true);
      return "found" as const;
    }).catch((error) => {
      console.warn("App update check failed", error);
      if (surfaceError) {
        setUpdateCheckError("Не вдалося перевірити оновлення. Перевірте інтернет і спробуйте ще раз");
      }
      return "error" as const;
    }).finally(() => {
      updateCheckInFlight.current = null;
      setUpdateChecking(false);
    });
    updateCheckInFlight.current = request;
    return request;
  }, []);

  useEffect(() => {
    let timer = 0;
    appMounted.current = true;
    const frame = window.requestAnimationFrame(() => {
      timer = window.setTimeout(() => {
        if (maintenanceStarted.current) return;
        maintenanceStarted.current = true;
        void maintainRuntime();
      }, 0);
    });
    return () => {
      appMounted.current = false;
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timer);
      if (maintenanceFollowUp.current !== null) window.clearTimeout(maintenanceFollowUp.current);
      if (startupFade.current !== null) window.clearTimeout(startupFade.current);
    };
  }, [maintainRuntime]);

  useEffect(() => {
    const timers = UPDATE_CHECK_DELAYS.map((delay, index) => window.setTimeout(() => {
      if (updateAvailableRef.current) return;
      void checkForAppUpdate(index === UPDATE_CHECK_DELAYS.length - 1);
    }, delay));
    const periodic = window.setInterval(() => {
      if (!updateAvailableRef.current) void checkForAppUpdate(false);
    }, UPDATE_CHECK_INTERVAL);
    const checkWhenOnline = () => {
      if (!updateAvailableRef.current) void checkForAppUpdate(true);
    };
    window.addEventListener("online", checkWhenOnline);
    return () => {
      timers.forEach(window.clearTimeout);
      window.clearInterval(periodic);
      window.removeEventListener("online", checkWhenOnline);
    };
  }, [checkForAppUpdate]);

  useEffect(() => {
    let disposed = false;
    let checking = false;
    if (!outputDir) {
      setOutputFreeSpace(null);
      return;
    }
    const refreshFolder = async () => {
      if (checking) return;
      checking = true;
      try {
        const bytes = await invoke<number>("folder_free_space", { path: outputDir });
        if (!disposed) setOutputFreeSpace(bytes);
      } catch {
        if (!disposed) setOutputFreeSpace(null);
        try {
          const fallback = await downloadDir();
          if (!disposed && fallback !== outputDir) {
            // The saved choice stays: the folder returns as soon as its disk does.
            setUnavailableOutputDir((current) => current || outputDir);
            setOutputDir((current) => current === outputDir ? fallback : current);
          }
        } catch {
          // Keep the unavailable path visible only if the system Downloads folder is also unavailable.
        }
      } finally {
        checking = false;
      }
    };
    refreshFolder();
    const timer = window.setInterval(refreshFolder, 2_500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [outputDir, job?.status]);

  useEffect(() => {
    if (!unavailableOutputDir) return;
    let disposed = false;
    const timer = window.setInterval(() => {
      void invoke<number>("folder_free_space", { path: unavailableOutputDir }).then(() => {
        if (disposed) return;
        setOutputDir(unavailableOutputDir);
        setUnavailableOutputDir(null);
      }).catch(() => undefined);
    }, 2_500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [unavailableOutputDir]);

  useEffect(() => {
    const destination = downloadQueue?.settings.outputDir;
    if (!destination || downloadQueue?.activeItemId) return;
    let disposed = false;
    let checking = false;
    const refreshQueueFolder = async () => {
      if (checking) return;
      checking = true;
      try {
        await invoke<number>("folder_free_space", { path: destination });
      } catch {
        try {
          const fallback = await downloadDir();
          await invoke<number>("folder_free_space", { path: fallback });
          if (!disposed) {
            updateQueue((current) => current && current.settings.outputDir === destination ? {
              ...current,
              settings: { ...current.settings, outputDir: fallback },
              updatedAt: new Date().toISOString(),
            } : current);
            setQueueNotice("Зовнішній диск або папка недоступні. Обрано системну папку Завантаження.");
          }
        } catch {
          // Keep the previous destination visible until either it or Downloads becomes available.
        }
      } finally {
        checking = false;
      }
    };
    void refreshQueueFolder();
    const timer = window.setInterval(refreshQueueFolder, 2_500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [downloadQueue?.activeItemId, downloadQueue?.settings.outputDir, updateQueue]);

  useEffect(() => {
    if (!startupBusy) return;
    const timer = window.setInterval(() => {
      setStartupProgress((current) => current < 90 ? Math.min(90, current + Math.max(0.8, (90 - current) * 0.06)) : current);
    }, 120);
    return () => window.clearInterval(timer);
  }, [startupBusy]);

  useEffect(() => {
    if (!historyRetryPending || active || pendingStart) return;
    if (!normalizeInputUrl(url)) {
      setHistoryRetryPending(null);
      setFormError("Збережене посилання більше не схоже на коректне");
      return;
    }
    setHistoryRetryPending(null);
    void beginDownload();
  }, [historyRetryPending, url, active, pendingStart]);

  useEffect(() => {
    const unlisten = listen<DownloadEvent>("download-event", ({ payload }) => {
      const queueItem = downloadQueueRef.current?.items.find((item) => item.jobId === payload.id) || null;
      if (shouldPlayCompletionSound(payload.kind) && !queueItem) {
        void invoke("play_completion_sound").catch(() => undefined);
      }
      if (payload.kind === "metadata") {
        const context = historyContexts.current.get(payload.id);
        if (context) {
          context.title = payload.title || context.title;
          context.thumbnail = payload.thumbnail || context.thumbnail;
          context.uploader = payload.uploader || context.uploader;
          context.extractor = payload.extractor || context.extractor;
        }
      }
      if (["completed", "failed", "cancelled", "auth_required"].includes(payload.kind)) {
        const context = historyContexts.current.get(payload.id);
        if (context) {
          const downloadedAt = new Date().toISOString();
          const outputs = payload.outputs || [];
          const multiple = outputs.length > 1;
          const additions = outputs.map((output, index): HistoryEntry => ({
            id: `${payload.id}:${index}`,
            sourceUrl: output.sourceUrl || context.sourceUrl,
            title: output.title || (multiple ? fileTitle(output.path) : context.title),
            thumbnail: output.thumbnail || (multiple ? null : context.thumbnail),
            cachedThumbnailPath: null,
            uploader: output.uploader || (multiple ? null : context.uploader),
            extractor: output.extractor || context.extractor,
            size: output.size,
            downloadedAt,
            path: output.path,
            available: true,
            settings: { ...context.settings, playlist: output.sourceUrl ? false : context.settings.playlist },
          }));
          if (payload.kind !== "completed") additions.push({
            id: `${payload.id}:attempt`, sourceUrl: context.sourceUrl, title: context.title,
            outcome: payload.kind as HistoryEntry["outcome"], message: payload.message || "Спробу перервано",
            thumbnail: null, cachedThumbnailPath: null, uploader: context.uploader,
            extractor: context.extractor, size: 0, downloadedAt, path: "", available: false,
            settings: context.settings,
          });
          setHistoryEntries((current) => limitHistoryDiagnostics([
            ...additions,
            ...current.filter((entry) => !additions.some((addition) => addition.id === entry.id || (addition.path && addition.path === entry.path))),
          ].slice(0, HISTORY_LIMIT)));
          const additionIds = new Set(additions.map((entry) => entry.id));
          void invoke<string>("download_diagnostics", { id: payload.id }).then((report) => {
            if (typeof report !== "string") return;
            setHistoryEntries((current) => limitHistoryDiagnostics(current.map((entry) => additionIds.has(entry.id)
              ? { ...entry, diagnostics: report.slice(0, 4000) } : entry)));
          }).catch(() => undefined);
          for (const addition of additions.filter((entry) => entry.thumbnail)) {
            const additionIds = new Set([addition.id]);
            void invoke<string>("cache_history_thumbnail", { url: addition.thumbnail })
              .then((cachedThumbnailPath) => {
                setHistoryEntries((current) => {
                  const next = applyHistoryThumbnailCache(current, additionIds, cachedThumbnailPath);
                  if (next === current && addition.thumbnail
                    && shouldDeleteUnusedHistoryThumbnail(current, addition.thumbnail, cachedThumbnailPath)) {
                    void invoke("delete_history_thumbnail", { path: cachedThumbnailPath }).catch(() => undefined);
                  }
                  return next;
                });
              })
              .catch(() => undefined);
          }
        }
      }
      if (["completed", "failed", "cancelled", "auth_required"].includes(payload.kind)) {
        historyContexts.current.delete(payload.id);
      }
      if (!queueItem && payload.storage?.availableSpace !== null && payload.storage?.availableSpace !== undefined) {
        setOutputFreeSpace(payload.storage.availableSpace);
      }
      if (queueItem) {
        updateQueue((current) => {
          if (!current) return current;
          let pauseQueue = false;
          const cancellationIntent = queueCancellationIntent.current;
          const items = current.items.map((item) => {
            if (item.id !== queueItem.id) return item;
            if (payload.kind === "metadata") return {
              ...item,
              title: payload.title || item.title,
              thumbnail: payload.thumbnail || item.thumbnail,
              uploader: payload.uploader || item.uploader,
              extractor: payload.extractor || item.extractor,
            };
            if (payload.kind === "storage_estimate") return {
              ...item,
              message: payload.message || item.message,
              storage: payload.storage ? {
                requiredSpace: payload.storage.requiredSpace,
                availableSpace: payload.storage.availableSpace,
                sufficient: payload.storage.sufficient,
              } : item.storage,
            };
            if (payload.kind === "progress") return { ...item, status: "downloading" as const, percent: payload.percent ?? item.percent, speed: payload.speed || item.speed, eta: payload.eta || item.eta, title: payload.title || item.title, message: payload.message || "Завантаження…" };
            if (payload.kind === "postprocess") return { ...item, status: "postprocessing" as const, message: payload.message || "Об’єднуємо потоки…" };
            if (payload.kind === "conversion_progress") return { ...item, status: "converting" as const, percent: payload.percent ?? item.percent, speed: payload.speed || item.speed, eta: payload.eta || item.eta, message: payload.message || "Створюємо сумісний файл…" };
            if (payload.kind === "retrying") return { ...item, status: "starting" as const, message: payload.message || "Повторюємо спробу…" };
            if (payload.kind === "completed") {
              const outputs = payload.outputs || [];
              return { ...item, status: "completed" as const, percent: 100, message: payload.message || "Готово", outputs, finalSize: outputs.reduce((sum, output) => sum + output.size, 0), errorCode: null };
            }
            if (payload.kind === "cancelled") {
              const status = cancellationIntent === "skip" ? "skipped" as const : "interrupted" as const;
              if (cancellationIntent !== "skip") pauseQueue = true;
              const outputs = payload.outputs || [];
              return { ...item, status, outputs, finalSize: outputs.reduce((sum, output) => sum + output.size, 0), message: status === "skipped" ? "Пропущено" : "Зупинено користувачем", jobId: null };
            }
            if (payload.kind === "failed" || payload.kind === "auth_required") {
              const blocking = isBlockingQueueFailure(payload.kind, payload.errorCode);
              pauseQueue = blocking;
              const outputs = payload.outputs || [];
              return { ...item, status: blocking ? "interrupted" as const : "failed" as const, outputs, finalSize: outputs.reduce((sum, output) => sum + output.size, 0), message: payload.message || "Не вдалося завантажити", errorCode: payload.errorCode || (payload.kind === "auth_required" ? "auth_required" : "unknown") };
            }
            return item;
          });
          if (pauseQueue) {
            const failedItem = items.find((item) => item.id === queueItem.id);
            setQueueMonitorOpen(true);
            setQueueAlert({
              title: payload.kind === "auth_required" || payload.errorCode === "auth_required" ? "Потрібен вхід" : payload.errorCode === "low_disk" ? "Замало місця" : "Чергу призупинено",
              message: payload.kind === "auth_required" || payload.errorCode === "auth_required"
                ? "Увійдіть на сайт у браузері, відкрийте налаштування пакетного завантаження, виберіть цей браузер і продовжте чергу."
                : failedItem?.message || "Виправте проблему та продовжте чергу.",
            });
            void invoke("request_user_attention").catch(() => undefined);
          }
          if (["completed", "failed", "cancelled", "auth_required"].includes(payload.kind)) queueCancellationIntent.current = null;
          return {
            ...current,
            status: pauseQueue ? "paused" : current.status,
            activeItemId: ["completed", "failed", "cancelled", "auth_required"].includes(payload.kind) ? null : current.activeItemId,
            items,
            updatedAt: new Date().toISOString(),
          };
        });
        return;
      }
      if (["completed", "failed", "auth_required"].includes(payload.kind)) {
        void invoke("request_user_attention").catch(() => undefined);
      }
      setJob((current) => {
        if (current?.id === payload.id) return applyDownloadEvent(current, payload);
        const buffered = pendingDownloadEvents.current.get(payload.id) || [];
        pendingDownloadEvents.current.set(payload.id, [...buffered.slice(-31), payload]);
        while (pendingDownloadEvents.current.size > 8) {
          const oldest = pendingDownloadEvents.current.keys().next().value;
          if (oldest === undefined) break;
          pendingDownloadEvents.current.delete(oldest);
        }
        return current;
      });
    });
    return () => { unlisten.then((dispose) => dispose()); };
  }, [updateQueue]);

  async function chooseFolder() {
    const selected = await open({ directory: true, multiple: false, title: "Оберіть папку для завантажень" });
    if (typeof selected === "string") {
      setUnavailableOutputDir(null);
      setOutputDir(selected);
      localStorage.setItem("outputDir", selected);
    }
  }

  async function contactSupport() {
    const subject = encodeURIComponent("Проблема з yt-dlp BD");
    try {
      await openUrl(`mailto:baldojnisyly@gmail.com?subject=${subject}`);
    } catch (error) {
      setFormError(`Не вдалося відкрити пошту: ${String(error)}`);
    }
  }

  function recordFailedStart(id: string, message: string) {
    const context = historyContexts.current.get(id);
    if (!context) return;
    const entry: HistoryEntry = { ...context, id: `${id}:attempt`, downloadedAt: new Date().toISOString(), outcome: "failed", message,
      path: "", size: 0, available: false, cachedThumbnailPath: null };
    setHistoryEntries((current) => limitHistoryDiagnostics([entry, ...current].slice(0, HISTORY_LIMIT)));
    void invoke<string>("download_diagnostics", {id}).then((report) => {
      if (typeof report !== "string") return;
      setHistoryEntries((current) => limitHistoryDiagnostics(current.map((item) => item.id === entry.id ? {...item, diagnostics: report.slice(0, 4000)} : item)));
    }).catch(() => undefined);
  }

  async function launchDownload(downloadUrl: string, parsedHost: string, cookiesBrowser?: string, playlist = playlistIntentToMultiItem(playlistIntent)) {
    const id = crypto.randomUUID();
    const normalizedDownloadUrl = normalizeInputUrl(downloadUrl) || downloadUrl;
    const thumbnail = youtubeThumbnailFromInput(normalizedDownloadUrl);
    const context: HistoryContext = {
      sourceUrl: normalizedDownloadUrl,
      title: parsedHost,
      thumbnail,
      uploader: null,
      extractor: parsedHost,
      settings: {
        outputDir,
        mode,
        quality,
        audioFormat,
        subtitles,
        playlist,
      },
    };
    historyContexts.current.set(id, context);
    const initialJob: Job = {
      id,
      url: normalizedDownloadUrl,
      title: parsedHost,
      status: "starting",
      percent: 0,
      speed: "—",
      eta: "—",
      message: cookiesBrowser ? `Вхід через ${cookiesBrowser}…` : "Підключення…",
      storage: null,
      playlist,
      outputFormat: mode === "video" ? "MP4" : audioFormat.toUpperCase(),
    };
    const buffered = pendingDownloadEvents.current.get(id) || [];
    pendingDownloadEvents.current.delete(id);
    setJob(buffered.reduce<Job | null>((current, event) => current ? applyDownloadEvent(current, event) : null, initialJob));
    try {
      await invoke<void>("start_download", {
        request: {
          id,
          url: normalizedDownloadUrl,
          outputDir,
          mode,
          quality,
          audioFormat,
          subtitles,
          multiItem: playlist,
          cookiesBrowser: cookiesBrowser || null,
          discoverySession: discoverySelections.current.get("single")?.pageUrl === normalizedDownloadUrl ? discoverySelections.current.get("single")?.sessionId : null,
        },
      });
      setUrl("");
    } catch (error) {
      recordFailedStart(id, String(error));
      historyContexts.current.delete(id);
      const message = String(error);
      setJob((current) => current?.id === id ? { ...current, status: "failed", message } : current);
    } finally {
      discoverySelections.current.delete("single");
    }
  }

  async function runPreflight(downloadUrl: string, parsedHost: string, cookiesBrowser?: string, playlist = playlistIntentToMultiItem(playlistIntent)) {
    setPendingStart(true);
    setFormError("");
    try {
      const normalized = normalizeInputUrl(downloadUrl);
      if (!normalized) throw new Error("Вставте коректне посилання");
      const result = await invoke<PreflightResult>("preflight_download", { url: normalized, outputDir });
      setOutputFreeSpace(result.availableSpace);
      if (!preflightAllowsStart(result)) {
        setFormError("Недостатньо вільного місця: перед стартом потрібно залишити щонайменше 500 МіБ резерву.");
        return;
      }
      await launchDownload(downloadUrl, parsedHost, cookiesBrowser, playlist);
    } catch (error) {
      setFormError(String(error));
    } finally {
      setPendingStart(false);
    }
  }

  async function beginDownload() {
    setFormError("");
    if (queuePreventsOtherWork(downloadQueueRef.current)) {
      setFormError("Дочекайтеся завершення пакетного завантаження або зупиніть його");
      return;
    }
    const normalized = normalizedInputUrl;
    const parsedHost = normalized ? hostFromInput(normalized) : null;
    if (!normalized || !parsedHost) {
      setFormError("Вставте коректне посилання на відео або аудіо");
      return;
    }
    if (!outputDir) {
      setFormError("Оберіть папку для завантаження");
      return;
    }
    if (!runtimeReady) {
      setFormError("Компоненти ще готуються. Зачекайте кілька секунд");
      return;
    }
    if (isRussianDomain(parsedHost) && !approvedVpnUrls.current.has(normalized)) {
      setPendingStart(true);
      try {
        const vpn = await invoke<VpnStatus>("check_vpn", { host: parsedHost });
        if (!vpn.detected) {
          setVpnWarning({ host: parsedHost, url: normalized, status: vpn, queueItemId: null });
          return;
        }
        approvedVpnUrls.current.add(normalized);
      } catch {
        setVpnWarning({
          host: parsedHost,
          url: normalized,
          status: { detected: false, interfaceName: null, confidence: "unknown", detail: "Не вдалося перевірити стан VPN" },
          queueItemId: null,
        });
        return;
      } finally {
        setPendingStart(false);
      }
    }

    await runPreflight(normalized, parsedHost, undefined, isWholeCollectionUrl(normalized) || playlistIntentToMultiItem(playlistIntent));
  }

  function clearUrl() {
    setUrl("");
    setFormError("");
    window.requestAnimationFrame(() => urlInputRef.current?.focus());
  }

  async function retryHistoryDownload(entry: HistoryEntry) {
    if (active || queueBusy || pendingStart || historyRetryPending || historyPreparationInFlight.current) return;
    historyPreparationInFlight.current = true;
    setPendingStart(true);
    setFormError("");
    let destination = entry.settings.outputDir || parentDirectory(entry.path);
    try {
      if (!destination) throw new Error("missing destination");
      await invoke<number>("folder_free_space", { path: destination });
    } catch {
      try {
        destination = await downloadDir();
      } catch {
        setHistoryRetryPending(null);
        historyPreparationInFlight.current = false;
        setPendingStart(false);
        setActiveView("download");
        setFormError("Не вдалося відкрити попередню папку або системну папку Завантаження");
        return;
      }
    }
    setUnavailableOutputDir(null);
    setOutputDir(destination);
    try { localStorage.setItem("outputDir", destination); } catch { /* Keep working in memory. */ }
    setMode(entry.settings.mode);
    setQuality(entry.settings.quality);
    setAudioFormat(entry.settings.audioFormat);
    setSubtitles(entry.settings.subtitles);
    setPlaylistIntent(entry.settings.playlist ? "playlist" : "single");
    cancelledDownloadRestore.current = { url: entry.sourceUrl, playlist: entry.settings.playlist };
    setUrl(entry.sourceUrl);
    setActiveView("download");
    // Publish the trigger in the same render as the complete restored form.
    setHistoryRetryPending(entry.id);
    historyPreparationInFlight.current = false;
    setPendingStart(false);
  }

  function dismissVpnWarning() {
    const queueItemId = vpnWarning?.queueItemId;
    setVpnWarning(null);
    if (queueItemId) {
      setQueueNotice("Пакетне завантаження призупинено: для цього посилання не підтверджено VPN.");
    } else {
      setFormError("Завантаження скасовано: VPN не підтверджено");
    }
  }

  function approveVpnWarning() {
    if (!vpnWarning) return;
    approvedVpnUrls.current.add(vpnWarning.url);
    const queueItemId = vpnWarning.queueItemId;
    setVpnWarning(null);
    if (queueItemId) {
      setQueueNotice("");
      updateQueue((current) => current ? { ...current, status: "running", updatedAt: new Date().toISOString() } : current);
      setActiveView("queue");
    } else {
      window.requestAnimationFrame(() => void beginDownload());
    }
  }

  async function retryWithCookies() {
    if (!job) return;
    const parsedHost = hostFromInput(job.url);
    if (!parsedHost) return;
    localStorage.setItem("cookieBrowser", cookieBrowser);
    await runPreflight(job.url, parsedHost, cookieBrowser, job.playlist);
  }

  async function cancelJob() {
    if (!job) return;
    const jobId = job.id;
    const cancelledUrl = job.url;
    const cancelledPlaylist = job.playlist;
    setCancelBusy(true);
    setCancelError("");
    try {
      await invoke("cancel_download", { id: jobId });
      // The process can still emit completed playlist files in its terminal event.
      // Retain the context until that event is persisted in History.
      cancelledDownloadRestore.current = { url: cancelledUrl, playlist: cancelledPlaylist };
      setUrl(cancelledUrl);
      setCancelConfirmOpen(false);
      // Keep the app busy (and awake) until the backend has reaped the child
      // and persisted the terminal event, including any completed playlist files.
      setJob((current) => current?.id === jobId && ["starting", "downloading", "postprocessing", "converting"].includes(current.status)
        ? { ...current, message: "Зупиняємо процес і завершуємо прибирання тимчасових файлів…" } : current);
    } catch (error) {
      setCancelError(String(error));
    } finally {
      setCancelBusy(false);
    }
  }

  function currentQueueSettings(): QueueSettings {
    return {
      outputDir,
      mode: "video",
      quality: "best",
      audioFormat: "mp3",
      subtitles: false,
      multiItem: false,
      cookiesBrowser: null,
    };
  }

  function addQueueText(raw = queueInput) {
    if (!raw.trim()) return;
    const current = downloadQueueRef.current;
    const result = commitQueueInput(current, currentQueueSettings(), raw);
    updateQueue(() => result.queue);
    setQueueInput("");
    setQueueNotice(result.rejected ? `Додано максимум ${QUEUE_LIMIT} посилань. Зайві рядки не потрапили в чергу.` : "");
  }

  function handleQueuePaste(event: ClipboardEvent<HTMLInputElement | HTMLTextAreaElement>) {
    const pasted = event.clipboardData.getData("text");
    if (!pasted.includes("\n")) return;
    event.preventDefault();
    const start = event.currentTarget.selectionStart ?? queueInput.length;
    const end = event.currentTarget.selectionEnd ?? start;
    addQueueText(`${queueInput.slice(0, start)}${pasted}${queueInput.slice(end)}`);
  }

  function changeQueueItem(id: string, value: string) {
    updateQueue((current) => current ? {
      ...current,
      status: current.status === "completed" ? "paused" : current.status,
      items: current.items.map((item) => item.id === id ? { ...item, url: value, status: "pending", message: "Очікує", errorCode: null, outputs: [], finalSize: 0, storage: null } : item),
      updatedAt: new Date().toISOString(),
    } : current);
  }

  function removeQueueItem(id: string) {
    updateQueue((current) => current ? {
      ...current,
      status: current.status === "completed" ? "paused" : current.status,
      items: current.items.filter((item) => item.id !== id),
      updatedAt: new Date().toISOString(),
    } : current);
  }

  function updateQueueSettings(patch: Partial<QueueSettings>) {
    updateQueue((current) => current ? {
      ...current,
      settings: { ...current.settings, ...patch },
      updatedAt: new Date().toISOString(),
    } : current);
  }

  async function chooseQueueFolder() {
    const selected = await open({ directory: true, multiple: false, title: "Оберіть папку для черги" });
    if (typeof selected === "string") updateQueueSettings({ outputDir: selected });
  }

  async function startQueue() {
    if (queueInput.trim()) addQueueText(queueInput);
    const current = downloadQueueRef.current;
    if (!current?.items.length || active || pendingStart || current.activeItemId || queueLaunchInFlight.current) return;
    if (runtimeBusy || !runtimeReady) {
      setQueueNotice("Компоненти ще готуються. Зачекайте кілька секунд.");
      return;
    }
    let destination = current.settings.outputDir;
    if (!destination) {
      setQueueNotice("Оберіть папку для завантажень.");
      return;
    }
    try {
      await invoke<number>("folder_free_space", { path: destination });
    } catch {
      try {
        destination = await downloadDir();
        await invoke<number>("folder_free_space", { path: destination });
        updateQueue((queue) => queue ? {
          ...queue,
          settings: { ...queue.settings, outputDir: destination },
          updatedAt: new Date().toISOString(),
        } : queue);
        setQueueNotice("Попередня папка недоступна. Використовуємо системну папку Завантаження.");
      } catch {
        setQueueNotice("Не вдалося відкрити обрану папку або системну папку Завантаження.");
        return;
      }
    }
    if (destination === current.settings.outputDir) setQueueNotice("");
    setQueueAlert(null);
    setQueueMonitorOpen(true);
    updateQueue((queue) => queue ? { ...queue, status: "running", updatedAt: new Date().toISOString() } : queue);
  }

  function pauseQueue() {
    updateQueue((current) => current ? { ...current, status: "paused", updatedAt: new Date().toISOString() } : current);
  }

  async function skipQueueItem() {
    const current = downloadQueueRef.current;
    const item = current?.items.find((candidate) => candidate.id === current.activeItemId);
    if (!item?.jobId) return;
    queueCancellationIntent.current = "skip";
    await invoke("cancel_download", { id: item.jobId }).catch((error) => setQueueNotice(String(error)));
  }

  async function stopQueue() {
    const current = downloadQueueRef.current;
    updateQueue((queue) => queue ? { ...queue, status: "paused", updatedAt: new Date().toISOString() } : queue);
    const item = current?.items.find((candidate) => candidate.id === current.activeItemId);
    if (item?.jobId) {
      queueCancellationIntent.current = "stop";
      await invoke("cancel_download", { id: item.jobId }).catch((error) => setQueueNotice(String(error)));
    }
  }

  function retryFailedQueueItems() {
    const current = downloadQueueRef.current;
    if (!current || active || current.activeItemId) return;
    const reset = new Map(resetQueueItemsForRetry(current.items).map((item) => [item.id, item]));
    updateQueue((queue) => queue ? {
      ...queue,
      status: "paused",
      items: queue.items.map((item) => reset.get(item.id) || item),
      activeItemId: null,
      updatedAt: new Date().toISOString(),
    } : queue);
    setQueueAlert(null);
    setQueueMonitorOpen(true);
    void startQueue();
  }

  function replayEntireQueue() {
    const current = downloadQueueRef.current;
    if (!current?.items.length || active || current.activeItemId) return;
    updateQueue((queue) => queue ? {
      ...queue,
      status: "paused",
      items: resetEntireQueueForReplay(queue.items),
      activeItemId: null,
      updatedAt: new Date().toISOString(),
    } : queue);
    setQueueAlert(null);
    setQueueMonitorOpen(true);
    void startQueue();
  }

  function createNewQueue() {
    if (queueHasActiveProcess(downloadQueueRef.current)) {
      setQueueNotice("Дочекайтеся завершення поточного файла або зупиніть його перед створенням нової черги.");
      return;
    }
    updateQueue(() => null);
    setQueueInput("");
    setQueueNotice("");
    setQueueAlert(null);
    setQueueMonitorOpen(false);
  }

  useEffect(() => {
    const queue = downloadQueue;
    if (!queue || queue.status !== "running" || queue.activeItemId || queueLaunchInFlight.current || active) return;
    const item = nextPendingQueueItem(queue.items);
    if (!item) {
      const failed = queue.items.filter((candidate) => candidate.status === "failed").length;
      const skipped = queue.items.filter((candidate) => candidate.status === "skipped").length;
      updateQueue((current) => current?.status === "running" ? { ...current, status: "completed", updatedAt: new Date().toISOString() } : current);
      if (shouldPlayQueueCompletionSound(queue.items)) {
        void invoke("play_completion_sound").catch(() => undefined);
      }
      void invoke("request_user_attention").catch(() => undefined);
      setQueueMonitorOpen(true);
      setQueueAlert({
        title: failed ? "Чергу завершено з помилками" : skipped ? "Чергу завершено з пропусками" : "Пакетне завантаження завершено",
        message: failed
          ? `Усі посилання оброблено. Не завантажено: ${failed + skipped}. Можна повторити всю чергу або лише проблемні рядки.`
          : skipped
            ? `Усі посилання оброблено. Пропущено: ${skipped}. Кнопка «Повторити чергу» знову запустить усі початкові посилання.`
            : "Усі посилання успішно завантажено. Результати вже зібрані в таблиці.",
      });
      return;
    }
    const normalized = normalizeHttpUrl(item.url);
    if (!normalized) {
      updateQueue((current) => current ? {
        ...current,
        items: current.items.map((candidate) => candidate.id === item.id ? { ...candidate, status: "failed", message: "Некоректне HTTP(S)-посилання", errorCode: "invalid_url" } : candidate),
        updatedAt: new Date().toISOString(),
      } : current);
      return;
    }
    const host = hostFromInput(normalized) || "Посилання";
    if (isRussianDomain(host) && !approvedVpnUrls.current.has(normalized)) {
      queueLaunchInFlight.current = true;
      void invoke<VpnStatus>("check_vpn", { host }).then((vpn) => {
        const latest = downloadQueueRef.current;
        if (!latest || latest.status !== "running" || latest.activeItemId || !latest.items.some((candidate) => candidate.id === item.id)) return;
        if (vpn.detected) {
          approvedVpnUrls.current.add(normalized);
          updateQueue((current) => current ? { ...current, updatedAt: new Date().toISOString() } : current);
          return;
        }
        updateQueue((current) => current ? { ...current, status: "paused", updatedAt: new Date().toISOString() } : current);
        setVpnWarning({ host, url: normalized, status: vpn, queueItemId: item.id });
        void invoke("request_user_attention").catch(() => undefined);
      }).catch(() => {
        const latest = downloadQueueRef.current;
        if (!latest || latest.status !== "running" || latest.activeItemId) return;
        updateQueue((current) => current ? { ...current, status: "paused", updatedAt: new Date().toISOString() } : current);
        setVpnWarning({
          host,
          url: normalized,
          status: { detected: false, interfaceName: null, confidence: "unknown", detail: "Не вдалося перевірити стан VPN" },
          queueItemId: item.id,
        });
        void invoke("request_user_attention").catch(() => undefined);
      }).finally(() => {
        queueLaunchInFlight.current = false;
      });
      return;
    }
    queueLaunchInFlight.current = true;
    const jobId = crypto.randomUUID();
    updateQueue((current) => current ? {
      ...current,
      activeItemId: item.id,
      items: current.items.map((candidate) => candidate.id === item.id ? { ...candidate, jobId, url: normalized, status: "starting", percent: 0, message: "Підключення…", attempt: candidate.attempt + 1 } : candidate),
      updatedAt: new Date().toISOString(),
    } : current);
    historyContexts.current.set(jobId, {
      sourceUrl: normalized,
      title: item.title || host,
      thumbnail: item.thumbnail,
      uploader: item.uploader,
      extractor: item.extractor || host,
      settings: {
        outputDir: queue.settings.outputDir,
        mode: queue.settings.mode,
        quality: queue.settings.quality,
        audioFormat: queue.settings.audioFormat,
        subtitles: queue.settings.subtitles,
        playlist: queue.settings.multiItem || isWholeCollectionUrl(normalized),
      },
    });
    void invoke<void>("start_download", {
      request: {
        id: jobId,
        url: normalized,
        outputDir: queue.settings.outputDir,
        mode: queue.settings.mode,
        quality: queue.settings.quality,
        audioFormat: queue.settings.audioFormat,
        subtitles: queue.settings.subtitles,
        multiItem: queue.settings.multiItem || isWholeCollectionUrl(normalized),
        cookiesBrowser: queue.settings.cookiesBrowser,
        discoverySession: discoverySelections.current.get(item.id)?.pageUrl === normalized ? discoverySelections.current.get(item.id)?.sessionId : null,
      },
    }).catch((error) => {
      recordFailedStart(jobId, String(error));
      historyContexts.current.delete(jobId);
      const message = String(error);
      const blocking = /вільн(ого|е) місц|folder|папк/i.test(message);
      updateQueue((current) => current ? {
        ...current,
        status: blocking ? "paused" : current.status,
        activeItemId: null,
        items: current.items.map((candidate) => candidate.id === item.id ? { ...candidate, jobId: null, status: blocking ? "interrupted" : "failed", message, errorCode: blocking ? "low_disk" : "start_failed" } : candidate),
        updatedAt: new Date().toISOString(),
      } : current);
      if (blocking) {
        setQueueMonitorOpen(true);
        setQueueAlert({ title: "Чергу призупинено", message });
        void invoke("request_user_attention").catch(() => undefined);
      }
    }).finally(() => {
      discoverySelections.current.delete(item.id);
      queueLaunchInFlight.current = false;
    });
  }, [active, downloadQueue, updateQueue]);

  async function installAppUpdate() {
    if (!update) return;
    setUpdateBusy(true);
    setUpdateInstallError("");
    setUpdateProgress(0);
    let downloaded = 0;
    let total = 0;
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") total = event.data.contentLength || 0;
        if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          if (total) setUpdateProgress(Math.round((downloaded / total) * 100));
        }
      });
      await invoke("allow_app_exit_once");
      try {
        await relaunch();
      } catch (error) {
        await invoke("cancel_app_exit_approval").catch(() => undefined);
        throw error;
      }
    } catch (error) {
      setUpdateInstallError(`Не вдалося встановити оновлення: ${String(error)}`);
      setUpdateBusy(false);
    }
  }

  async function dismissAppClose() {
    setCloseRequest(null);
    setCloseBusy(false);
    setCloseError("");
    await invoke("dismiss_app_close_request").catch(() => undefined);
  }

  async function confirmAppClose() {
    setCloseBusy(true);
    setCloseError("");
    try {
      await invoke("confirm_app_close");
    } catch (error) {
      setCloseError(`Не вдалося закрити застосунок: ${String(error)}`);
      setCloseBusy(false);
    }
  }

  return (
    <div className="app-shell">
      {startupBusy && <StartupOverlay progress={startupProgress} message={startupMessage} />}
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark"><img src={brandLogo} alt="" /></div>
          <div><small>yt-dlp BD</small><strong>Baldojnyi Downloader</strong></div>
        </div>
        <nav>
          <button className={`nav-item ${activeView === "download" ? "active" : ""}`} onClick={() => setActiveView("download")}><span className="nav-index">01</span>Завантажити{activeView === "download" && <Icon name="chevron" />}</button>
          <button className={`nav-item batch-download ${activeView === "queue" ? "active" : ""}`} onClick={() => setActiveView("queue")}><span className="nav-index">02</span><span className="nav-label">Пакетне завантаження</span>{activeView === "queue" && <Icon name="chevron" />}</button>
          <button className={`nav-item ${activeView === "history" ? "active" : ""}`} onClick={() => setActiveView("history")}><span className="nav-index">03</span>Історія{activeView === "history" && <Icon name="chevron" />}</button>
        </nav>
        <div className="runtime-panel">
          <div className="runtime-heading"><span>Компоненти</span><small>{runtimeBusy ? "перевірка" : runtimeReady ? "готові" : "очікуємо"}</small></div>
          <RuntimeRow label="yt-dlp" component={runtime?.ytDlp} loading={runtimeStage === "ytDlp"} retry={runtimeFailures.has("ytDlp") || runtime?.ytDlp.installed === false ? () => retryRuntimeComponent("ytDlp") : undefined} />
          <RuntimeRow label="ffmpeg" component={runtime?.ffmpeg} loading={runtimeStage === "ffmpeg"} retry={runtimeFailures.has("ffmpeg") || runtime?.ffmpeg.installed === false ? () => retryRuntimeComponent("ffmpeg") : undefined} />
          <RuntimeRow label="Deno" component={runtime?.deno} loading={runtimeStage === "deno"} retry={runtimeFailures.has("deno") || runtime?.deno.installed === false ? () => retryRuntimeComponent("deno") : undefined} />
          {runtimeError && <p className="runtime-error">{runtimeError}</p>}
        </div>
        <div className="support-card">
          <span>Щось не працює?</span>
          <button onClick={contactSupport}>baldojnisyly@gmail.com <span aria-hidden="true">↗</span></button>
        </div>
        <div className="sidebar-footer"><span className="status-dot"/>Версія {APP_RELEASE_VERSION} · {isWindows ? "Windows x64" : "Apple Silicon"}</div>
      </aside>

      <main ref={mainContentRef} className={`main-content ${mainScrollable || activeView !== "download" ? "is-scrollable" : "is-fixed"}`}>
        <RecoveryNotice ready={!startupBusy && !anyProcessActive && !runtimeBusy}/>
        <header className="topbar">
          {activeView === "download"
            ? <div><p className="eyebrow">ІНСТРУМЕНТ / 01</p><h1>Завантажити</h1><p>Відео, аудіо й субтитри з YouTube, Instagram, Twitter — і майже звідусіль, звідки захочете.</p></div>
            : activeView === "queue"
              ? <div><p className="eyebrow">ІНСТРУМЕНТ / 02</p><h1 className="batch-title">Пакетне завантаження</h1><p>За потреби розгорніть налаштування, оберіть формат, якість і папку для завантаження. Потім додайте до 50 посилань із будь-яких платформ.</p></div>
              : <div><p className="eyebrow">АРХІВ / 03</p><h1>Історія</h1><p>Завантажені файли й місця, де вони збережені.</p></div>}
          {update
            ? <button className="update-pill" disabled={updateBusy} onClick={() => setUpdatePromptOpen(true)}><Icon name="refresh" size={16}/>{updateBusy ? `Оновлення ${updateProgress}%` : `Доступна v${update.version}`}</button>
            : updateCheckError
              ? <button className="update-pill update-retry" disabled={updateChecking} onClick={() => void checkForAppUpdate(true)}><Icon name="refresh" size={16}/>{updateChecking ? "Перевіряємо…" : "Повторити перевірку"}</button>
              : null}
        </header>

        {activeView !== "queue" && downloadQueue && ["running", "paused"].includes(downloadQueue.status) && <button className="queue-strip" onClick={() => setQueueMonitorOpen(true)}>
          <span><strong>Пакетне завантаження: {downloadQueue.status === "running" ? "працює" : downloadQueue.activeItemId ? "завершує поточний файл" : "призупинене"}</strong><small>{queueProgress(downloadQueue.items).done} із {queueProgress(downloadQueue.items).total} оброблено</small></span>
          <span>{queueProgress(downloadQueue.items).percent}% →</span>
        </button>}

        {activeView === "download" && <section className="download-card" aria-busy={runtimeBusy}>
          <div className="download-card-content" inert={runtimeBusy ? true : undefined}>
          <label className="field-label" htmlFor="media-url">Посилання</label>
          <div className={`url-field ${urlState}`}>
            <Icon name="link" />
            <input ref={urlInputRef} id="media-url" value={url} onChange={(event) => setUrl(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !active) beginDownload(); }} placeholder="https://youtube.com/watch?v=…" autoFocus />
            {url && <button type="button" className="url-clear" aria-label="Очистити посилання" title="Очистити посилання" onClick={clearUrl}><Icon name="x" size={17}/></button>}
            {urlState === "valid" && <span className="valid-check" aria-label="Коректне посилання"><Icon name="check" size={15}/></span>}
            {urlState === "invalid" && <span className="invalid-check" aria-label="Некоректне посилання"><Icon name="x" size={15}/></span>}
          </div>
          {urlState === "invalid" && <div className="url-error"><Icon name="x" size={14}/><span>Вставте повне HTTP або HTTPS посилання</span></div>}

          <DownloadCollectionIntent url={url} intent={playlistIntent} onIntentChange={setPlaylistIntent} />

          <div className="choice-grid">
            <div>
              <span className="field-label">Що завантажуємо</span>
              <div className="segmented">
                <button className={mode === "video" ? "selected" : ""} onClick={() => setMode("video")}>Відео</button>
                <button className={mode === "audio" ? "selected" : ""} onClick={() => setMode("audio")}>Тільки аудіо</button>
              </div>
            </div>
            <div>
              <label className="field-label" htmlFor="quality">{mode === "video" ? "Якість" : "Формат аудіо"}</label>
              {mode === "video" ? (
                <select id="quality" value={quality} onChange={(event) => setQuality(event.target.value)}><option value="best">Найкраща доступна (MP4)</option><option value="2160">До 4K</option><option value="1080">До 1080p</option><option value="720">До 720p</option><option value="480">До 480p</option></select>
              ) : (
                <select id="quality" value={audioFormat} onChange={(event) => setAudioFormat(event.target.value)}><option value="mp3">MP3</option><option value="m4a">M4A</option><option value="opus">Opus</option><option value="wav">WAV</option></select>
              )}
            </div>
          </div>

          <label className="toggle-row"><span><strong>Завантажити субтитри</strong><small>Українські та англійські, якщо доступні</small></span><input type="checkbox" checked={subtitles} onChange={(event) => setSubtitles(event.target.checked)}/><i/></label>

          <div className="folder-row">
            <div className="folder-copy"><span className="field-label">Зберегти у</span><div className="folder-value"><Icon name="folder" size={18}/><span>{outputDir || "Папку не обрано"}</span></div>{outputDir && <small className="folder-space">{outputFreeSpace === null ? "Перевіряємо вільне місце…" : `Вільно ${formatByteSize(outputFreeSpace)}`}</small>}</div>
            <button className="secondary-button" onClick={chooseFolder}>Змінити</button>
          </div>
          {unavailableOutputDir && <div className="inline-notice" role="status">Папка «{unavailableOutputDir}» зараз недоступна. Поки зберігаємо у «Завантаження»; вона повернеться, щойно диск з’явиться.</div>}

          {formError && <div className="inline-error">{formError}</div>}
          <button className="primary-button" disabled={urlState !== "valid" || anyProcessActive || queueBusy || pendingStart || runtimeBusy || !runtimeReady} onClick={() => beginDownload()}><Icon name="download" />{runtimeBusy ? "Встановлюємо компоненти…" : pendingStart ? "Починаємо…" : anyProcessActive || queueBusy ? "Інше завантаження триває" : "Завантажити"}</button>
          <p className="legal-note">Завантажуйте лише матеріали, на які маєте відповідні права.</p>
          </div>
        </section>}

        {activeView === "queue" && <QueueView
          queue={downloadQueue}
          input={queueInput}
          notice={queueNotice}
          runtimeReady={runtimeReady && !runtimeBusy}
          singleDownloadActive={Boolean(active)}
          onInput={setQueueInput}
          onPaste={handleQueuePaste}
          onAdd={() => addQueueText()}
          onChangeItem={changeQueueItem}
          onRemoveItem={removeQueueItem}
          onSettings={updateQueueSettings}
          onChooseFolder={chooseQueueFolder}
          onStart={() => void startQueue()}
          onPause={pauseQueue}
          onResume={() => void startQueue()}
          onSkip={() => void skipQueueItem()}
          onStop={() => void stopQueue()}
          onRetry={retryFailedQueueItems}
          onReplay={replayEntireQueue}
          onNew={createNewQueue}
        />}

        {activeView === "history" && <HistoryView
          entries={historyEntries}
          retryingId={historyRetryPending}
          retryDisabled={Boolean(active || queueBusy || pendingStart)}
          onRetry={retryHistoryDownload}
          onClear={() => {
            if (!window.confirm("Очистити історію? Завантажені файли залишаться на диску.")) return;
            historyEntriesRef.current = [];
            setHistoryEntries([]);
            void invoke("clear_history_thumbnail_cache").catch(() => undefined);
          }}
        />}

      </main>

      <MediaCandidatePicker busy={anyProcessActive || pendingStart || runtimeBusy || startupBusy}
      isCurrent={(candidate) => job?.id === candidate.jobId || Boolean(downloadQueueRef.current?.items.some((item) => item.jobId === candidate.jobId && ["failed", "interrupted"].includes(item.status)))}
      onSelected={({ pageUrl, jobId, sessionId }) => {
        const item = downloadQueueRef.current?.items.find((item) => item.jobId === jobId && ["failed", "interrupted"].includes(item.status));
        if (item) {
          discoverySelections.current.set(item.id, { sessionId, pageUrl });
          updateQueue((queue) => queue ? { ...queue, status: "paused", items: queue.items.map((i) => i.id === item.id ? { ...i, status: "pending", errorCode: null, percent: 0, jobId: null } : i) } : queue);
          setQueueNotice("Джерело обрано. Натисніть Продовжити, щоб повторити завантаження.");
          setQueueMonitorOpen(true);
        } else if (job?.id === jobId) {
          discoverySelections.current.set("single", { sessionId, pageUrl });
          setJob(null); setUrl(pageUrl); setHistoryRetryPending("discovery");
        }
      }}/>
      {runtimeBusy && !startupBusy && <RuntimePreparationOverlay stage={runtimeStage} runtime={runtime} progress={runtimeInstallProgress} pending={runtimePending} />}

      {job && job.status !== "auth_required" && <div className="download-progress-backdrop" role="presentation">
        <DialogSurface className={`job-card ${job.status}`} labelledBy="download-progress-title" onEscape={!active ? () => setJob(null) : undefined}>
          {job.status === "completed" ? (
            <button type="button" className="job-icon status-action" aria-label="Закрити повідомлення про завершення" onClick={() => setJob(null)}><Icon name="check"/></button>
          ) : job.status === "failed" ? (
            <button type="button" className="job-icon status-action" aria-label="Закрити повідомлення про помилку" onClick={() => setJob(null)}><Icon name="x" size={24}/></button>
          ) : (
            <div className="job-icon" aria-label={`Етап ${jobStageNumber(job.status)}`}><span className="job-stage"><small>ЕТАП</small><strong>{jobStageNumber(job.status)}</strong></span></div>
          )}
          <div className="job-body">
            <div className="job-title"><div ref={jobTitleTextRef}><strong id="download-progress-title">{job.title}</strong><small>{job.status === "completed" ? job.message || "Готово" : job.status === "failed" ? job.message || "Помилка завантаження" : job.status === "cancelled" ? "Скасовано" : job.message}</small></div><span style={{ fontSize: `${jobValueFontSize}px` }}>{job.status === "postprocessing" || job.status === "converting" ? job.outputFormat : `${Math.round(job.percent)}%`}</span></div>
            <div className="job-source" title={job.url}><Icon name="link" size={12}/><span>{job.url}</span></div>
            {job.status === "completed" && job.outputPaths?.[0] && <button type="button" className="secondary-button job-reveal" onClick={() => void revealItemInDir(job.outputPaths![0]).catch((error) => setFormError(String(error)))}><Icon name="folder" size={16}/>Показати у папці</button>}
            {!active && job.status === "failed" && <DownloadTools url={job.url} jobId={job.id} canDiscover={canOfferBrowserDiscovery(job.errorCode)}/>}
            {job.storage && <div className={`storage-estimate ${job.storage.sufficient === false ? "warning" : job.storage.sufficient === true ? "ready" : "unknown"}`}>
              {job.storage.sufficient === false ? <Icon name="shield" size={15}/> : <span className="storage-estimate-dot"/>}
              <span>{job.storage.sufficient === false
                ? `Може не вистачити місця: вільно ${job.storage.availableSpace === null ? "невідомо" : formatByteSize(job.storage.availableSpace)} · потрібно до ${formatByteSize(job.storage.requiredSpace)}. Завантаження продовжується.`
                : `${job.storage.availableSpace === null ? "Вільне місце невідоме" : `Вільно ${formatByteSize(job.storage.availableSpace)}`} · потрібно до ${formatByteSize(job.storage.requiredSpace)}`}</span>
            </div>}
            <div className={`progress-track ${job.status === "postprocessing" ? "indeterminate" : ""}`} role="progressbar" aria-label={job.status === "postprocessing" ? "Об’єднання завантажених потоків" : job.status === "converting" ? "Конвертація відео" : "Завантаження відео"} aria-valuenow={job.status === "postprocessing" ? undefined : Math.round(job.percent)}><span style={job.status === "postprocessing" ? undefined : { width: `${job.percent}%` }}/></div>
            <div className="job-meta"><span>{job.status === "converting" ? `${Math.round(job.percent)}% · ${job.speed}` : job.speed}</span><span>{job.status === "postprocessing" ? "Готуємо файл до кодування" : job.eta !== "—" ? `Залишилось ${job.eta}` : ""}</span></div>
          </div>
          {active && <button className="stop-button" aria-label="Зупинити" onClick={() => { setCancelError(""); setCancelConfirmOpen(true); }}><Icon name="stop" size={18}/></button>}
          {!active && <button className="stop-button close" aria-label="Закрити" onClick={() => setJob(null)}><Icon name="x" size={18}/></button>}
        </DialogSurface>
      </div>}

      {queueMonitorOpen && downloadQueue && downloadQueue.status !== "draft" && <QueueProgressModal
        queue={downloadQueue}
        alert={queueAlert}
        singleDownloadActive={Boolean(active)}
        onClose={() => { setQueueMonitorOpen(false); setQueueAlert(null); }}
        onPause={pauseQueue}
        onResume={() => void startQueue()}
        onSkip={() => void skipQueueItem()}
        onStop={() => void stopQueue()}
        onRetry={retryFailedQueueItems}
        onReplay={replayEntireQueue}
        onNew={createNewQueue}
      />}

      {cancelConfirmOpen && job && active && <div className="modal-backdrop" role="presentation">
        <DialogSurface className="modal" labelledBy="cancel-title" describedBy="cancel-description" onEscape={!cancelBusy ? () => setCancelConfirmOpen(false) : undefined}>
          <div className="modal-icon cancel"><Icon name="stop" size={28}/></div>
          <p className="eyebrow">АКТИВНЕ ЗАВАНТАЖЕННЯ</p>
          <h2 id="cancel-title">Скасувати процес?</h2>
          <p id="cancel-description">Завантаження або конвертація зупиниться, а тимчасові файли буде прибрано. Це вікно закриється, щойно все завершиться.</p>
          {cancelError && <div className="modal-detail cancel-error">{cancelError}</div>}
          <div className="modal-actions"><button className="secondary-button" disabled={cancelBusy} onClick={() => setCancelConfirmOpen(false)}>Продовжити</button><button className="cancel-confirm-button" disabled={cancelBusy} onClick={cancelJob}>{cancelBusy ? "Зупиняємо…" : "Так, скасувати"}</button></div>
        </DialogSurface>
      </div>}

      {update && updatePromptOpen && !startupBusy && !active && !queueBusy && <div className="modal-backdrop update-modal-backdrop" role="presentation">
        <DialogSurface className="modal update-modal" labelledBy="update-title" describedBy="update-description" onEscape={!updateBusy ? () => setUpdatePromptOpen(false) : undefined}>
          <div className="modal-icon update"><Icon name="refresh" size={30}/></div>
          <p className="eyebrow">ДОСТУПНЕ ОНОВЛЕННЯ</p>
          <h2 id="update-title">Нова версія {update.version}</h2>
          <p id="update-description">Зараз встановлена версія <strong>{update.currentVersion}</strong>. Оновлення завантажиться, встановиться й автоматично перезапустить застосунок.</p>
          {update.body && <UpdateReleaseNotes notes={update.body}/>}
          {updateBusy && <div className="update-download-progress" role="progressbar" aria-label="Завантаження оновлення" aria-valuemin={0} aria-valuemax={100} aria-valuenow={updateProgress}><span style={{ width: `${Math.max(2, updateProgress)}%` }}/></div>}
          {updateInstallError && <div className="modal-detail cancel-error">{updateInstallError}</div>}
          <div className="modal-actions"><button className="secondary-button" disabled={updateBusy} onClick={() => setUpdatePromptOpen(false)}>Пізніше</button><button className="update-install-button" disabled={updateBusy} onClick={installAppUpdate}>{updateBusy ? `Оновлюємо ${updateProgress}%` : "Оновити зараз"}</button></div>
        </DialogSurface>
      </div>}

      {vpnWarning && <div className="modal-backdrop" role="presentation">
        <DialogSurface className="modal" labelledBy="vpn-title" describedBy="vpn-description" onEscape={dismissVpnWarning}>
          <div className="modal-icon"><Icon name="shield" size={29}/></div>
          <p className="eyebrow">ПЕРЕВІРКА БЕЗПЕКИ</p>
          <h2 id="vpn-title">VPN не виявлено</h2>
          <p id="vpn-description">Ви збираєтесь завантажити матеріал із <strong>{vpnWarning.host}</strong>. Не вдалося підтвердити, що VPN увімкнений.</p>
          <div className="modal-detail">{vpnWarning.status.detail}</div>
          <div className="modal-actions"><button className="secondary-button" onClick={dismissVpnWarning}>Скасувати</button><button className="warning-button" onClick={approveVpnWarning}>Продовжити без VPN</button></div>
        </DialogSurface>
      </div>}

      {job?.status === "auth_required" && <div className="modal-backdrop" role="presentation">
        <DialogSurface className="modal" labelledBy="auth-title" describedBy="auth-description" onEscape={() => setJob(null)}>
          <div className="modal-icon auth"><Icon name="link" size={28}/></div>
          <p className="eyebrow">ПОТРІБНА АВТОРИЗАЦІЯ</p>
          <h2 id="auth-title">Увійдіть через браузер</h2>
          <p id="auth-description">Сайт просить підтвердити вік, акаунт або що ви не бот. Увійдіть на цей сайт у браузері та виберіть його нижче.</p>
          <label className="browser-picker">Ваш браузер<select value={cookieBrowser} onChange={(event) => setCookieBrowser(event.target.value)}><option value="edge">Microsoft Edge</option><option value="chrome">Google Chrome</option><option value="firefox">Firefox</option><option value="brave">Brave</option><option value="vivaldi">Vivaldi</option></select></label>
          <div className="modal-detail">yt-dlp прочитає cookies безпосередньо з обраного браузера лише для повторної спроби. yt-dlp BD не експортує їх у файл і не зберігає.{isWindows && " На Windows Firefox зазвичай читається надійніше; оберіть саме той браузер, де ви вже увійшли на сайт."}</div>
          <div className="modal-actions"><button className="secondary-button" onClick={() => setJob(null)}>Скасувати</button><button className="warning-button auth" disabled={pendingStart} onClick={retryWithCookies}>{pendingStart ? "Перевіряємо…" : "Повторити з cookies"}</button></div>
        </DialogSurface>
      </div>}

      {closeRequest && <div className="modal-backdrop app-close-backdrop" role="presentation">
        <DialogSurface className="modal app-close-modal" labelledBy="app-close-title" describedBy="app-close-description" onEscape={!closeBusy ? () => void dismissAppClose() : undefined}>
          <div className="modal-icon app-close"><Icon name="x" size={30}/></div>
          <p className="eyebrow">ЗАКРИТТЯ ЗАСТОСУНКУ</p>
          <h2 id="app-close-title">Ти точно хочеш закрити?</h2>
          <p id="app-close-description">{closeActivities.length
            ? "Зараз у застосунку ще виконуються процеси. Якщо закрити його, вони зупиняться, а незавершені файли можуть не зберегтися."
            : "Активних процесів зараз немає, але ми все одно перепитуємо, щоб застосунок не закрився випадково."}</p>
          {closeActivities.length > 0 && <div className="app-close-processes" aria-label="Активні процеси">
            <strong>Зараз працює</strong>
            <ul>{closeActivities.map((activity) => <li key={activity}>{activity}</li>)}</ul>
          </div>}
          {closeError && <div className="modal-detail cancel-error">{closeError}</div>}
          <div className="modal-actions"><button className="secondary-button" disabled={closeBusy} onClick={() => void dismissAppClose()}>Ні, залишитись</button><button className="app-close-confirm-button" disabled={closeBusy} onClick={() => void confirmAppClose()}>{closeBusy ? "Закриваємо…" : "Так, закрити"}</button></div>
        </DialogSurface>
      </div>}

    </div>
  );
}

function UpdateReleaseNotes({ notes }: { notes: string }) {
  return <div className="modal-detail update-notes">
    {notes.split(/\r?\n/).map((line, index) => {
      const heading = line.match(/^#{1,3}\s+(.+)$/);
      if (heading) return <strong className="update-notes-heading" key={index}>{heading[1]}</strong>;
      const item = line.match(/^[-*]\s+(.+)$/);
      if (item) return <div className="update-notes-item" key={index}><span>•</span><span>{item[1]}</span></div>;
      if (!line.trim()) return null;
      return <p key={index}>{line}</p>;
    })}
  </div>;
}

export { applyHistoryFileStatuses, applyHistoryThumbnailCache, groupHistoryEntries, historyPaths, limitHistoryDiagnostics, parseHistoryStorage, shouldCacheHistoryThumbnail, shouldDeleteUnusedHistoryThumbnail, type HistoryEntry, type HistoryFileStatus } from "./history";
export { shouldShowRuntimeRetry } from "./RuntimeViews";
export default App;
