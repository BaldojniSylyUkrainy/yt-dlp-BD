import { useCallback, useEffect, useRef, useState, type ClipboardEvent } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { downloadDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import brandLogo from "./assets/logo-call-me-hands.png";
import loadingHandOne from "./assets/logo-call-me-hand-frame-1.png";
import loadingHandTwo from "./assets/logo-call-me-hand-frame-2.png";
import loadingHandThree from "./assets/logo-call-me-hand-frame-3.png";
import packageMetadata from "../package.json";
import {
  appendQueueUrls,
  canEditQueueItem,
  nextPendingQueueItem,
  normalizeHttpUrl,
  parseQueueStorage,
  QUEUE_LIMIT,
  QUEUE_STORAGE_KEY,
  queueHasActiveProcess,
  queuePreventsOtherWork,
  queueProgress,
  resetQueueItemsForRetry,
  type DownloadQueue,
  type QueueItem,
  type QueueSettings,
} from "./queue";
import "./App.css";

const APP_RELEASE_VERSION = packageMetadata.releaseVersion;
const HISTORY_STORAGE_KEY = "downloadHistory.v1";
const HISTORY_LIMIT = 500;
export const UPDATE_CHECK_DELAYS = [0, 5_000, 30_000] as const;
const UPDATE_CHECK_INTERVAL = 6 * 60 * 60 * 1_000;

export function shouldPlayCompletionSound(kind: DownloadEvent["kind"]): boolean {
  return kind === "completed";
}

type ComponentStatus = {
  installed: boolean;
  version: string | null;
  path: string | null;
  managed: boolean;
};

type RuntimeStatus = {
  ytDlp: ComponentStatus;
  ffmpeg: ComponentStatus;
  deno: ComponentStatus;
  runtimeDir: string;
  platform: string;
};

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
};

export type HistoryEntry = {
  id: string;
  sourceUrl: string;
  title: string;
  thumbnail: string | null;
  cachedThumbnailPath: string | null;
  uploader: string | null;
  extractor: string | null;
  size: number;
  downloadedAt: string;
  path: string;
  available: boolean;
  settings: HistoryDownloadSettings;
};

export type HistoryDownloadSettings = {
  outputDir: string;
  mode: "video" | "audio";
  quality: string;
  audioFormat: string;
  subtitles: boolean;
  playlist: boolean;
};

export type HistoryFileStatus = {
  path: string;
  available: boolean;
  size: number | null;
};

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
  title: string;
  itemCount: number;
  intermediateSize: number | null;
  finalOutputSize: number | null;
  protectedReserve: number;
  requiredSpace: number | null;
  availableSpace: number;
  confidence: "exact" | "approximate" | "unknown";
  sufficient: boolean;
};

export function preflightAllowsStart(result: PreflightResult): boolean {
  return result.sufficient;
}

export function preflightConfidenceLabel(confidence: PreflightResult["confidence"]): string {
  if (confidence === "exact") return "Точні дані";
  if (confidence === "approximate") return "Орієнтовно";
  return "Розмір частково невідомий";
}

export function runtimeInstallCommand(stage: Exclude<RuntimeStage, null>): string {
  if (stage === "ytDlp") return "install_ytdlp";
  if (stage === "ffmpeg") return "install_ffmpeg";
  return "install_deno";
}

export function defaultCookieBrowser(platform: string, stored: string | null): string {
  if (stored && !(platform === "windows" && stored === "safari")) return stored;
  return platform === "windows" ? "edge" : "safari";
}

export function isCurrentProbe(currentSequence: number, responseSequence: number): boolean {
  return currentSequence === responseSequence;
}

export type Job = {
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
};

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
      title: payload.title || payload.message || current.title,
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
      speed: payload.speed || "VideoToolbox",
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
    percent: payload.kind === "completed" ? 100 : current.percent,
    message: payload.kind === "auth_required" ? "Потрібен вхід через браузер" : payload.message || current.message,
  };
}

type RuntimeStage = "ytDlp" | "ffmpeg" | "deno" | null;

type RuntimeInstallProgress = {
  component: Exclude<RuntimeStage, null>;
  downloaded: number;
  total: number | null;
};

type MediaPreview = {
  title: string;
  thumbnail: string | null;
  duration: number | null;
  durationIsTotal: boolean;
  uploader: string | null;
  extractor: string | null;
  webpageUrl: string | null;
  itemCount: number | null;
};

type ProbeState = "idle" | "checking" | "valid" | "unverified" | "invalid";

type IconName =
  | "download"
  | "clock"
  | "settings"
  | "folder"
  | "link"
  | "shield"
  | "check"
  | "x"
  | "refresh"
  | "list"
  | "stop"
  | "chevron";

function Icon({ name, size = 20 }: { name: IconName; size?: number }) {
  const paths: Record<IconName, React.ReactNode> = {
    download: <><path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/></>,
    clock: <><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></>,
    settings: <><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.86 2.86-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 .6 1.7 1.7 0 0 0-.4 1.1V21H9.5v-.1A1.7 1.7 0 0 0 8.4 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.86-2.86.06-.06A1.7 1.7 0 0 0 4 15a1.7 1.7 0 0 0-.6-1 1.7 1.7 0 0 0-1.1-.4H2.2V9.5h.1A1.7 1.7 0 0 0 4 8.4a1.7 1.7 0 0 0-.34-1.88l-.06-.06L6.46 3.6l.06.06A1.7 1.7 0 0 0 8.4 4a1.7 1.7 0 0 0 1-.6 1.7 1.7 0 0 0 .4-1.1v-.1h4.1v.1A1.7 1.7 0 0 0 15 4a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.86 2.86-.06.06A1.7 1.7 0 0 0 19.4 8.4a1.7 1.7 0 0 0 .6 1 1.7 1.7 0 0 0 1.1.4h.1v4.1h-.1A1.7 1.7 0 0 0 19.4 15Z"/></>,
    folder: <path d="M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2h8.5A1.5 1.5 0 0 1 21 8.5v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5Z"/>,
    link: <><path d="M10 13a5 5 0 0 0 7.1.1l2-2a5 5 0 0 0-7.1-7.1l-1.1 1.1"/><path d="M14 11a5 5 0 0 0-7.1-.1l-2 2A5 5 0 0 0 12 20l1.1-1.1"/></>,
    shield: <><path d="M12 3 5 6v5c0 4.6 2.8 8 7 10 4.2-2 7-5.4 7-10V6Z"/><path d="m9 12 2 2 4-4"/></>,
    check: <path d="m5 12 4 4L19 6"/>,
    x: <><path d="m7 7 10 10"/><path d="M17 7 7 17"/></>,
    refresh: <><path d="M20 7v5h-5"/><path d="M4 17v-5h5"/><path d="M6.1 8A7 7 0 0 1 18 6l2 6M4 12l2 6a7 7 0 0 0 11.9-2"/></>,
    list: <><path d="M8 6h12M8 12h12M8 18h12"/><path d="M4 6h.01M4 12h.01M4 18h.01"/></>,
    stop: <rect x="6" y="6" width="12" height="12" rx="2"/>,
    chevron: <path d="m9 18 6-6-6-6"/>,
  };
  return <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">{paths[name]}</svg>;
}

function hostFromInput(value: string): string | null {
  try {
    return new URL(/^https?:\/\//i.test(value.trim()) ? value.trim() : `https://${value.trim()}`).hostname.toLowerCase();
  } catch {
    return null;
  }
}

function normalizeInputUrl(value: string): string | null {
  try {
    return new URL(/^https?:\/\//i.test(value.trim()) ? value.trim() : `https://${value.trim()}`).toString();
  } catch {
    return null;
  }
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
    return ["/playlist/", "/playlists/", "/sets/", "/album/", "/albums/", "/collection/", "/collections/", "/showcase/"].some((segment) => path.includes(segment));
  } catch {
    return false;
  }
}

function shortVersion(value: string | null): string {
  if (!value) return "Не встановлено";
  return value.replace(/^nightly@/, "").split("\n")[0];
}

function formatDuration(value: number | null): string | null {
  if (value === null || !Number.isFinite(value)) return null;
  const seconds = Math.max(0, Math.round(value));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  return hours
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(remainder).padStart(2, "0")}`
    : `${minutes}:${String(remainder).padStart(2, "0")}`;
}

function formatByteSize(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "—";
  const units = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${new Intl.NumberFormat("uk-UA", { maximumFractionDigits: unit === 0 ? 0 : 1 }).format(size)} ${units[unit]}`;
}

function historyDayKey(value: string): string {
  const date = new Date(value);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

function historyDayLabel(key: string, now = new Date()): string {
  const today = historyDayKey(now.toISOString());
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (key === today) return "Сьогодні";
  if (key === historyDayKey(yesterday.toISOString())) return "Вчора";
  return new Intl.DateTimeFormat("uk-UA", { day: "numeric", month: "long", year: "numeric" })
    .format(new Date(`${key}T12:00:00`));
}

export function groupHistoryEntries(entries: HistoryEntry[], now = new Date()) {
  const groups = new Map<string, HistoryEntry[]>();
  [...entries]
    .sort((left, right) => Date.parse(right.downloadedAt) - Date.parse(left.downloadedAt))
    .forEach((entry) => {
      const key = historyDayKey(entry.downloadedAt);
      groups.set(key, [...(groups.get(key) || []), entry]);
    });
  return Array.from(groups, ([key, items]) => ({ key, label: historyDayLabel(key, now), items }));
}

function fileTitle(path: string): string {
  const filename = path.split(/[\\/]/).filter(Boolean).pop() || path;
  return filename.replace(/\.[^.]+$/, "") || filename;
}

function parentDirectory(path: string): string {
  const separator = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return separator > 0 ? path.slice(0, separator) : "";
}

function fallbackHistorySettings(path: string): HistoryDownloadSettings {
  const extension = path.split(".").pop()?.toLowerCase() || "";
  const audio = ["mp3", "m4a", "opus", "wav"].includes(extension);
  return {
    outputDir: parentDirectory(path),
    mode: audio ? "audio" : "video",
    quality: "best",
    audioFormat: audio ? extension : "mp3",
    subtitles: false,
    playlist: false,
  };
}

function nonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function normalizeHistorySettings(value: unknown, path: string): HistoryDownloadSettings {
  const fallback = fallbackHistorySettings(path);
  const settings = value && typeof value === "object" ? value as Record<string, unknown> : {};
  const quality = nonEmptyString(settings.quality);
  const audioFormat = nonEmptyString(settings.audioFormat)?.toLowerCase() || null;
  return {
    outputDir: nonEmptyString(settings.outputDir) || fallback.outputDir,
    mode: settings.mode === "video" || settings.mode === "audio" ? settings.mode : fallback.mode,
    quality: quality && ["best", "2160", "1080", "720", "480"].includes(quality) ? quality : "best",
    audioFormat: audioFormat && ["mp3", "m4a", "opus", "wav"].includes(audioFormat)
      ? audioFormat
      : fallback.audioFormat,
    subtitles: typeof settings.subtitles === "boolean" ? settings.subtitles : false,
    playlist: typeof settings.playlist === "boolean" ? settings.playlist : false,
  };
}

function normalizeHistoryEntry(value: unknown): HistoryEntry | null {
  if (!value || typeof value !== "object") return null;
  const entry = value as Record<string, unknown>;
  const id = nonEmptyString(entry.id);
  const path = nonEmptyString(entry.path);
  const title = nonEmptyString(entry.title);
  const sourceUrl = nonEmptyString(entry.sourceUrl);
  const downloadedAt = nonEmptyString(entry.downloadedAt);
  if (!id || !path || !title || !sourceUrl || !downloadedAt || !Number.isFinite(Date.parse(downloadedAt))) {
    return null;
  }
  const normalizedSourceUrl = normalizeInputUrl(sourceUrl);
  if (!normalizedSourceUrl || !/^https?:/i.test(normalizedSourceUrl)) return null;
  const size = typeof entry.size === "number" && Number.isFinite(entry.size) && entry.size >= 0
    ? entry.size
    : 0;
  return {
    id,
    sourceUrl: normalizedSourceUrl,
    title,
    thumbnail: nonEmptyString(entry.thumbnail),
    cachedThumbnailPath: nonEmptyString(entry.cachedThumbnailPath),
    uploader: nonEmptyString(entry.uploader),
    extractor: nonEmptyString(entry.extractor),
    size,
    downloadedAt,
    path,
    available: typeof entry.available === "boolean" ? entry.available : true,
    settings: normalizeHistorySettings(entry.settings, path),
  };
}

export function parseHistoryStorage(raw: string | null): HistoryEntry[] {
  try {
    const parsed: unknown = JSON.parse(raw || "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.flatMap((entry) => {
      const normalized = normalizeHistoryEntry(entry);
      return normalized ? [normalized] : [];
    }).slice(0, HISTORY_LIMIT);
  } catch {
    return [];
  }
}

function loadHistory(): HistoryEntry[] {
  try {
    return parseHistoryStorage(localStorage.getItem(HISTORY_STORAGE_KEY));
  } catch {
    return [];
  }
}

export function historyPaths(entries: HistoryEntry[]): string[] {
  return Array.from(new Set(entries.map((entry) => entry.path)));
}

export function applyHistoryFileStatuses(entries: HistoryEntry[], statuses: HistoryFileStatus[]): HistoryEntry[] {
  const byPath = new Map(statuses.map((status) => [status.path, status]));
  let changed = false;
  const next = entries.map((entry) => {
    const status = byPath.get(entry.path);
    if (!status) return entry;
    const size = status.size ?? entry.size;
    if (entry.available === status.available && entry.size === size) return entry;
    changed = true;
    return { ...entry, available: status.available, size };
  });
  return changed ? next : entries;
}

export function shouldCacheHistoryThumbnail(kind: DownloadEvent["kind"], outputs: DownloadOutput[] | null, thumbnail: string | null): boolean {
  return kind === "completed" && Boolean(outputs?.length) && Boolean(thumbnail);
}

export function applyHistoryThumbnailCache(entries: HistoryEntry[], targetIds: Set<string>, cachedThumbnailPath: string): HistoryEntry[] {
  let changed = false;
  const next = entries.map((entry) => {
    if (!targetIds.has(entry.id) || entry.cachedThumbnailPath === cachedThumbnailPath) return entry;
    changed = true;
    return { ...entry, cachedThumbnailPath };
  });
  return changed ? next : entries;
}

export function shouldDeleteUnusedHistoryThumbnail(entries: HistoryEntry[], sourceThumbnail: string, cachedThumbnailPath: string): boolean {
  return !entries.some((entry) => entry.thumbnail === sourceThumbnail || entry.cachedThumbnailPath === cachedThumbnailPath);
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
  const probeSequence = useRef(0);
  const approvedVpnUrls = useRef(new Set<string>());
  const cancelledDownloadRestore = useRef<{ url: string; playlist: boolean } | null>(null);
  const mainContentRef = useRef<HTMLElement>(null);
  const jobTitleTextRef = useRef<HTMLDivElement>(null);
  const pendingDownloadEvents = useRef(new Map<string, DownloadEvent[]>());
  const historyContexts = useRef(new Map<string, HistoryContext>());
  const [runtime, setRuntime] = useState<RuntimeStatus | null>(null);
  const [runtimeBusy, setRuntimeBusy] = useState(false);
  const [runtimeStage, setRuntimeStage] = useState<RuntimeStage>(null);
  const [runtimeError, setRuntimeError] = useState("");
  const [runtimeFailures, setRuntimeFailures] = useState(new Set<Exclude<RuntimeStage, null>>());
  const [runtimeInstallProgress, setRuntimeInstallProgress] = useState<RuntimeInstallProgress | null>(null);
  const [url, setUrl] = useState("");
  const urlInputRef = useRef<HTMLInputElement>(null);
  const [activeView, setActiveView] = useState<"download" | "queue" | "history">("download");
  const [historyEntries, setHistoryEntries] = useState<HistoryEntry[]>([]);
  const historyEntriesRef = useRef(historyEntries);
  const [historyHydrated, setHistoryHydrated] = useState(false);
  const [historyRetryPending, setHistoryRetryPending] = useState<string | null>(null);
  const [outputDir, setOutputDir] = useState("");
  const [outputFreeSpace, setOutputFreeSpace] = useState<number | null>(null);
  const [mode, setMode] = useState<"video" | "audio">("video");
  const [quality, setQuality] = useState("best");
  const [audioFormat, setAudioFormat] = useState("mp3");
  const [subtitles, setSubtitles] = useState(false);
  const [playlistIntent, setPlaylistIntent] = useState<"single" | "playlist">("single");
  const [job, setJob] = useState<Job | null>(null);
  const [formError, setFormError] = useState("");
  const [vpnWarning, setVpnWarning] = useState<{ host: string; url: string; status: VpnStatus; queueItemId: string | null } | null>(null);
  const [vpnDecisionVersion, setVpnDecisionVersion] = useState(0);
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
  const [probeState, setProbeState] = useState<ProbeState>("idle");
  const [preview, setPreview] = useState<MediaPreview | null>(null);
  const [probeError, setProbeError] = useState("");
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
  const queueLaunchInFlight = useRef(false);
  const queueCancellationIntent = useRef<"skip" | "stop" | null>(null);
  const sleepPreventionChain = useRef<Promise<unknown>>(Promise.resolve());

  const active = job && ["starting", "downloading", "postprocessing", "converting"].includes(job.status);
  const queueProcessActive = queueHasActiveProcess(downloadQueue);
  const queueBusy = queuePreventsOtherWork(downloadQueue);
  const anyProcessActive = Boolean(active || queueProcessActive);
  const runtimeReady = Boolean(runtime?.ytDlp.installed && runtime?.ffmpeg.installed && runtime?.deno.installed);
  const isWindows = runtime?.platform === "windows";
  const quickThumbnail = youtubeThumbnailFromInput(url);
  const multiItemCandidate = isLikelyMultiItemUrl(url);

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
          const restoredQueue = parseQueueStorage(localStorage.getItem(QUEUE_STORAGE_KEY), fallbackSettings);
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
    try {
      if (downloadQueue) localStorage.setItem(QUEUE_STORAGE_KEY, JSON.stringify(downloadQueue));
      else localStorage.removeItem(QUEUE_STORAGE_KEY);
    } catch {
      // A queue can continue in memory even when local persistence is unavailable.
    }
  }, [downloadQueue, queueHydrated]);

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
    if (!isWindows || cookieBrowser !== "safari") return;
    setCookieBrowser("edge");
    localStorage.setItem("cookieBrowser", "edge");
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
    const now = Date.now();
    const schedules = [
      { command: "update_ytdlp", key: "runtimeUpdate.ytDlp", interval: 24 * 60 * 60 * 1000 },
      { command: "install_ffmpeg", key: "runtimeUpdate.ffmpeg", interval: 7 * 24 * 60 * 60 * 1000 },
      { command: "install_deno", key: "runtimeUpdate.deno", interval: 7 * 24 * 60 * 60 * 1000 },
    ];
    const due = schedules.filter(({ key, interval }) => now - Number(localStorage.getItem(key) || 0) >= interval);
    if (!due.length) return;
    const results = await Promise.allSettled(due.map(({ command }) => invoke<void>(command)));
    results.forEach((result, index) => {
      if (result.status === "fulfilled") localStorage.setItem(due[index].key, String(now));
    });
    invoke<RuntimeStatus>("runtime_status").then(setRuntime).catch(() => undefined);
  }, []);

  const maintainRuntime = useCallback(async () => {
    setStartupBusy(true);
    setStartupProgress(18);
    setStartupMessage("Перевіряємо локальні компоненти…");
    setRuntimeError("");
    const issues: string[] = [];
    try {
      const status = await invoke<RuntimeStatus>("runtime_status");
      setRuntime(status);
      setStartupProgress(62);
      const missing = [
        { stage: "ytDlp" as const, label: "yt-dlp", installed: status.ytDlp.installed, command: "install_ytdlp" },
        { stage: "ffmpeg" as const, label: "ffmpeg", installed: status.ffmpeg.installed, command: "install_ffmpeg" },
        { stage: "deno" as const, label: "Deno", installed: status.deno.installed, command: "install_deno" },
      ].filter(({ installed }) => !installed);
      if (!missing.length) {
        setRuntimeBusy(false);
        window.setTimeout(() => refreshManagedComponents(), 800);
        return;
      }
      setRuntimeBusy(true);
      for (const [index, component] of missing.entries()) {
        setRuntimeStage(component.stage);
        setStartupMessage(`Встановлюємо ${component.label}…`);
        setStartupProgress(62 + Math.round((index / missing.length) * 30));
        try {
          await invoke<void>(component.command);
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
      setRuntimeBusy(false);
      setStartupProgress(100);
      setStartupMessage("Готово");
      window.setTimeout(() => setStartupBusy(false), 320);
    }
  }, [refreshManagedComponents]);

  async function retryRuntimeComponent(stage: Exclude<RuntimeStage, null>) {
    const command = runtimeInstallCommand(stage);
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
      localStorage.setItem(`runtimeUpdate.${stage}`, String(Date.now()));
    } catch (error) {
      setRuntimeError(String(error));
      setRuntimeFailures((current) => new Set(current).add(stage));
    } finally {
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
    const frame = window.requestAnimationFrame(() => {
      timer = window.setTimeout(() => {
        if (maintenanceStarted.current) return;
        maintenanceStarted.current = true;
        void maintainRuntime();
      }, 0);
    });
    return () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timer);
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
          if (!disposed) {
            setOutputDir((current) => {
              if (current !== outputDir) return current;
              localStorage.setItem("outputDir", fallback);
              return fallback;
            });
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
    const value = url.trim();
    const sequence = ++probeSequence.current;
    setPreview(null);
    setProbeError("");
    if (!value) {
      setProbeState("idle");
      return;
    }
    if (!runtime?.ytDlp.installed) {
      setProbeState("idle");
      return;
    }
    const parsedHost = hostFromInput(value);
    const normalized = normalizeInputUrl(value);
    if (!parsedHost || !normalized) {
      setProbeState("invalid");
      setProbeError("Це не схоже на посилання");
      return;
    }
    setProbeState("checking");
    let softDeadline = 0;
    const timer = window.setTimeout(async () => {
      if (isRussianDomain(parsedHost) && !approvedVpnUrls.current.has(normalized)) {
        try {
          const vpn = await invoke<VpnStatus>("check_vpn", { host: parsedHost });
          if (!isCurrentProbe(probeSequence.current, sequence)) return;
          if (!vpn.detected) {
            setProbeState("idle");
            setVpnWarning({ host: parsedHost, url: normalized, status: vpn, queueItemId: null });
            return;
          }
          approvedVpnUrls.current.add(normalized);
        } catch {
          if (!isCurrentProbe(probeSequence.current, sequence)) return;
          setProbeState("idle");
          setVpnWarning({
            host: parsedHost,
            url: normalized,
            status: { detected: false, interfaceName: null, confidence: "unknown", detail: "Не вдалося перевірити стан VPN" },
            queueItemId: null,
          });
          return;
        }
      }
      softDeadline = window.setTimeout(() => {
        if (!isCurrentProbe(probeSequence.current, sequence)) return;
        setProbeError("Не вдалося швидко визначити доступність. Можна спробувати завантажити");
        setProbeState("unverified");
      }, 4_000);
      try {
        const result = await invoke<MediaPreview>("probe_url", { probeId: crypto.randomUUID(), url: normalized, playlist: playlistIntent === "playlist" });
        if (!isCurrentProbe(probeSequence.current, sequence)) return;
        setPreview(result);
        setProbeState("valid");
      } catch (error) {
        if (!isCurrentProbe(probeSequence.current, sequence)) return;
        setProbeError(String(error));
        setProbeState("unverified");
      } finally {
        window.clearTimeout(softDeadline);
      }
    }, 450);
    return () => {
      window.clearTimeout(timer);
      window.clearTimeout(softDeadline);
    };
  }, [url, runtime?.ytDlp.installed, playlistIntent, vpnDecisionVersion]);

  useEffect(() => {
    if (!historyRetryPending || active || pendingStart) return;
    if (probeState === "invalid") {
      setHistoryRetryPending(null);
      setFormError("Збережене посилання більше не схоже на коректне");
      return;
    }
    if (probeState !== "valid" && probeState !== "unverified") return;
    setHistoryRetryPending(null);
    void beginDownload();
  }, [historyRetryPending, probeState, active, pendingStart]);

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
      if (payload.kind === "completed" && payload.outputs?.length) {
        const context = historyContexts.current.get(payload.id);
        if (context) {
          const downloadedAt = new Date().toISOString();
          const multiple = payload.outputs.length > 1;
          const additions = payload.outputs.map((output, index): HistoryEntry => ({
            id: `${payload.id}:${index}`,
            sourceUrl: context.sourceUrl,
            title: multiple ? fileTitle(output.path) : context.title,
            thumbnail: context.thumbnail,
            cachedThumbnailPath: null,
            uploader: context.uploader,
            extractor: context.extractor,
            size: output.size,
            downloadedAt,
            path: output.path,
            available: true,
            settings: context.settings,
          }));
          setHistoryEntries((current) => [
            ...additions,
            ...current.filter((entry) => !additions.some((addition) => addition.path === entry.path)),
          ].slice(0, HISTORY_LIMIT));
          if (shouldCacheHistoryThumbnail(payload.kind, payload.outputs, context.thumbnail)) {
            const additionIds = new Set(additions.map((entry) => entry.id));
            void invoke<string>("cache_history_thumbnail", { url: context.thumbnail })
              .then((cachedThumbnailPath) => {
                setHistoryEntries((current) => {
                  const next = applyHistoryThumbnailCache(current, additionIds, cachedThumbnailPath);
                  if (next === current && context.thumbnail
                    && shouldDeleteUnusedHistoryThumbnail(current, context.thumbnail, cachedThumbnailPath)) {
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
            if (payload.kind === "progress") return { ...item, status: "downloading" as const, percent: payload.percent ?? item.percent, speed: payload.speed || item.speed, eta: payload.eta || item.eta, title: payload.title || item.title, message: "Завантаження…" };
            if (payload.kind === "postprocess") return { ...item, status: "postprocessing" as const, message: payload.message || "Об’єднуємо потоки…" };
            if (payload.kind === "conversion_progress") return { ...item, status: "converting" as const, percent: payload.percent ?? item.percent, speed: payload.speed || item.speed, eta: payload.eta || item.eta, message: payload.message || "Створюємо сумісний файл…" };
            if (payload.kind === "retrying") return { ...item, status: "starting" as const, message: payload.message || "Повторюємо спробу…" };
            if (payload.kind === "completed") {
              const outputs = payload.outputs || [];
              return { ...item, status: "completed" as const, percent: 100, message: "Готово", outputs, finalSize: outputs.reduce((sum, output) => sum + output.size, 0), errorCode: null };
            }
            if (payload.kind === "cancelled") {
              const status = cancellationIntent === "skip" ? "skipped" as const : "interrupted" as const;
              if (cancellationIntent !== "skip") pauseQueue = true;
              return { ...item, status, message: status === "skipped" ? "Пропущено" : "Зупинено користувачем", jobId: null };
            }
            if (payload.kind === "failed" || payload.kind === "auth_required") {
              const blocking = payload.errorCode === "low_disk" || payload.errorCode === "rate_limited" || payload.kind === "auth_required";
              pauseQueue = blocking;
              return { ...item, status: blocking ? "interrupted" as const : "failed" as const, message: payload.message || "Не вдалося завантажити", errorCode: payload.errorCode || (payload.kind === "auth_required" ? "auth_required" : "unknown"), jobId: null };
            }
            return item;
          });
          if (pauseQueue) {
            const failedItem = items.find((item) => item.id === queueItem.id);
            setQueueAlert({
              title: payload.kind === "auth_required" ? "Потрібен вхід" : payload.errorCode === "low_disk" ? "Замало місця" : "Чергу призупинено",
              message: payload.kind === "auth_required"
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

  async function launchDownload(downloadUrl: string, parsedHost: string, preflightResult: PreflightResult, cookiesBrowser?: string, playlist = playlistIntent === "playlist") {
    const id = crypto.randomUUID();
    const normalizedDownloadUrl = normalizeInputUrl(downloadUrl) || downloadUrl;
    const thumbnail = preview?.thumbnail || quickThumbnail;
    const context: HistoryContext = {
      sourceUrl: normalizedDownloadUrl,
      title: preview?.title || parsedHost,
      thumbnail,
      uploader: preview?.uploader || null,
      extractor: preview?.extractor || parsedHost,
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
      url: downloadUrl,
      title: parsedHost,
      status: "starting",
      percent: 0,
      speed: "—",
      eta: "—",
      message: cookiesBrowser ? `Вхід через ${cookiesBrowser}…` : "Підключення…",
      storage: preflightResult.requiredSpace === null ? null : {
        estimatedSize: preflightResult.finalOutputSize || 0,
        requiredSpace: preflightResult.requiredSpace,
        availableSpace: preflightResult.availableSpace,
        sufficient: preflightResult.sufficient,
      },
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
          url: /^https?:\/\//i.test(downloadUrl.trim()) ? downloadUrl.trim() : `https://${downloadUrl.trim()}`,
          outputDir,
          mode,
          quality,
          audioFormat,
          subtitles,
          multiItem: playlist,
          cookiesBrowser: cookiesBrowser || null,
          expectedRequiredSpace: preflightResult.requiredSpace,
        },
      });
      setUrl("");
    } catch (error) {
      historyContexts.current.delete(id);
      const message = String(error);
      setJob((current) => current?.id === id ? { ...current, status: "failed", message } : current);
    }
  }

  async function runPreflight(downloadUrl: string, parsedHost: string, cookiesBrowser?: string, playlist = playlistIntent === "playlist") {
    setPendingStart(true);
    setFormError("");
    try {
      const normalized = normalizeInputUrl(downloadUrl);
      if (!normalized) throw new Error("Вставте коректне посилання");
      const result = await invoke<PreflightResult>("preflight_download", {
        request: {
          url: normalized,
          outputDir,
          mode,
          quality,
          audioFormat,
          multiItem: playlist,
          title: preview?.title || null,
          duration: preview?.duration || null,
          itemCount: preview?.itemCount || null,
          durationIsTotal: preview?.durationIsTotal ?? null,
        },
      });
      setOutputFreeSpace(result.availableSpace);
      if (!preflightAllowsStart(result)) {
        setFormError(result.requiredSpace === null
          ? "Недостатньо вільного місця: перед стартом потрібно залишити щонайменше 500 МіБ резерву."
          : `Недостатньо вільного місця: потрібно ${formatByteSize(result.requiredSpace)}, доступно ${formatByteSize(result.availableSpace)}.`);
        return;
      }
      await launchDownload(downloadUrl, parsedHost, result, cookiesBrowser, playlist);
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
    const parsedHost = hostFromInput(url);
    if (!parsedHost) {
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
    if (probeState !== "valid" && probeState !== "unverified") {
      setFormError(probeState === "checking" ? "Зачекайте кілька секунд, поки триває швидка перевірка" : "Вставте коректне посилання");
      return;
    }

    const normalized = normalizeInputUrl(url);
    if (!normalized) {
      setFormError("Вставте коректне посилання");
      return;
    }
    if (isRussianDomain(parsedHost) && !approvedVpnUrls.current.has(normalized)) {
      setFormError("Спочатку завершіть перевірку VPN для цього посилання");
      return;
    }

    await runPreflight(url, parsedHost);
  }

  function clearUrl() {
    probeSequence.current += 1;
    setUrl("");
    setPreview(null);
    setProbeState("idle");
    setProbeError("");
    setFormError("");
    window.requestAnimationFrame(() => urlInputRef.current?.focus());
  }

  async function retryHistoryDownload(entry: HistoryEntry) {
    if (active || queueBusy || historyRetryPending) return;
    setHistoryRetryPending(entry.id);
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
        setActiveView("download");
        setFormError("Не вдалося відкрити попередню папку або системну папку Завантаження");
        return;
      }
    }
    setOutputDir(destination);
    localStorage.setItem("outputDir", destination);
    setMode(entry.settings.mode);
    setQuality(entry.settings.quality);
    setAudioFormat(entry.settings.audioFormat);
    setSubtitles(entry.settings.subtitles);
    setPlaylistIntent(entry.settings.playlist ? "playlist" : "single");
    cancelledDownloadRestore.current = { url: entry.sourceUrl, playlist: entry.settings.playlist };
    setPreview(null);
    setProbeState("idle");
    setUrl(entry.sourceUrl);
    setVpnDecisionVersion((current) => current + 1);
    setActiveView("download");
  }

  function dismissVpnWarning() {
    const queueItemId = vpnWarning?.queueItemId;
    setVpnWarning(null);
    if (queueItemId) {
      setQueueNotice("Пакетне завантаження призупинено: для цього посилання не підтверджено VPN.");
    } else {
      setProbeState("invalid");
      setProbeError("Перевірку посилання скасовано");
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
      setVpnDecisionVersion((current) => current + 1);
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
      historyContexts.current.delete(jobId);
      cancelledDownloadRestore.current = { url: cancelledUrl, playlist: cancelledPlaylist };
      setUrl(cancelledUrl);
      setCancelConfirmOpen(false);
      setJob((current) => current?.id === jobId ? null : current);
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
    const result = appendQueueUrls(current?.items || [], raw);
    const now = new Date().toISOString();
    updateQueue(() => ({
      version: 1,
      status: current?.status === "completed" ? "draft" : current?.status || "draft",
      settings: current?.settings || currentQueueSettings(),
      items: result.items,
      activeItemId: current?.activeItemId || null,
      createdAt: current?.createdAt || now,
      updatedAt: now,
    }));
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
    const current = downloadQueueRef.current;
    if (!current?.items.length || active || current.activeItemId || queueLaunchInFlight.current) return;
    if (!runtimeReady) {
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
    void startQueue();
  }

  useEffect(() => {
    const queue = downloadQueue;
    if (!queue || queue.status !== "running" || queue.activeItemId || queueLaunchInFlight.current || active) return;
    const item = nextPendingQueueItem(queue.items);
    if (!item) {
      updateQueue((current) => current?.status === "running" ? { ...current, status: "completed", updatedAt: new Date().toISOString() } : current);
      void invoke("play_completion_sound").catch(() => undefined);
      void invoke("request_user_attention").catch(() => undefined);
      setQueueAlert({ title: "Пакетне завантаження завершено", message: "Усі посилання оброблено. Результати вже зібрані в таблиці." });
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
        playlist: queue.settings.multiItem,
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
        multiItem: queue.settings.multiItem,
        cookiesBrowser: queue.settings.cookiesBrowser,
        expectedRequiredSpace: null,
      },
    }).catch((error) => {
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
        setQueueAlert({ title: "Чергу призупинено", message });
        void invoke("request_user_attention").catch(() => undefined);
      }
    }).finally(() => {
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
      await relaunch();
    } catch (error) {
      setUpdateInstallError(`Не вдалося встановити оновлення: ${String(error)}`);
      setUpdateBusy(false);
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

        {activeView !== "queue" && downloadQueue && ["running", "paused"].includes(downloadQueue.status) && <button className="queue-strip" onClick={() => setActiveView("queue")}>
          <span><strong>Пакетне завантаження: {downloadQueue.status === "running" ? "працює" : downloadQueue.activeItemId ? "завершує поточний файл" : "призупинене"}</strong><small>{queueProgress(downloadQueue.items).done} із {queueProgress(downloadQueue.items).total} оброблено</small></span>
          <span>{queueProgress(downloadQueue.items).percent}% →</span>
        </button>}

        {activeView === "download" && <section className="download-card" aria-busy={runtimeBusy}>
          {runtimeBusy && <RuntimePreparationOverlay stage={runtimeStage} runtime={runtime} progress={runtimeInstallProgress} />}
          <div className="download-card-content" inert={runtimeBusy ? true : undefined}>
          <label className="field-label" htmlFor="media-url">Посилання</label>
          <div className={`url-field ${probeState}`}>
            <Icon name="link" />
            <input ref={urlInputRef} id="media-url" value={url} onChange={(event) => setUrl(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !active) beginDownload(); }} placeholder="https://youtube.com/watch?v=…" autoFocus />
            {url && <button type="button" className="url-clear" aria-label="Очистити посилання" title="Очистити посилання" onClick={clearUrl}><Icon name="x" size={17}/></button>}
            {probeState === "checking" && <span className="url-checking" aria-label="Перевіряємо посилання"><i/></span>}
            {probeState === "valid" && <span className="valid-check" aria-label="Відео доступне"><Icon name="check" size={15}/></span>}
            {probeState === "unverified" && <span className="unverified-check" aria-label="Не вдалося підтвердити посилання">!</span>}
            {probeState === "invalid" && <span className="invalid-check" aria-label="Посилання недоступне"><Icon name="x" size={15}/></span>}
          </div>
          {probeState === "checking" && quickThumbnail && <div className="media-preview checking">
            <img src={quickThumbnail} alt="" />
            <div><strong>Зчитуємо назву відео…</strong><p>YouTube</p><small><span className="inline-spinner"/>Швидко перевіряємо доступність</small></div>
          </div>}
          {preview && probeState === "valid" && <div className="media-preview">
            {preview.thumbnail ? <img src={preview.thumbnail} alt="" /> : <div className="media-preview-placeholder"><Icon name="check"/></div>}
            <div><strong>{preview.title}</strong><p>{[preview.uploader, preview.duration ? `${formatDuration(preview.duration)}${preview.durationIsTotal ? "" : " на елемент"}` : null, preview.itemCount && preview.itemCount > 1 ? `${preview.itemCount} елементів` : null, preview.extractor].filter(Boolean).join(" · ")}</p><small><Icon name="check" size={13}/>Посилання доступне; формати перевіримо перед стартом</small></div>
          </div>}
          {probeState === "unverified" && <div className="url-warning" title={probeError}><span aria-hidden="true">!</span><span>Не вдалося швидко підтвердити посилання. yt-dlp все одно може спробувати його завантажити.</span></div>}
          {probeState === "invalid" && <div className="url-error"><Icon name="x" size={14}/><span>{probeError || "yt-dlp не може завантажити це посилання"}</span></div>}

          {multiItemCandidate && <div className="collection-intent">
            <span className="field-label">Що взяти з добірки</span>
            <div className="segmented">
              <button type="button" className={playlistIntent === "single" ? "selected" : ""} onClick={() => setPlaylistIntent("single")}>Лише це відео</button>
              <button type="button" className={playlistIntent === "playlist" ? "selected" : ""} onClick={() => setPlaylistIntent("playlist")}>Уся добірка</button>
            </div>
            <small>{playlistIntent === "single" ? "За замовчуванням завантажиться лише поточний ролик." : "Завантажаться всі доступні елементи з безпечними паузами між ними."}</small>
          </div>}

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

          {formError && <div className="inline-error">{formError}</div>}
          <button className="primary-button" disabled={(probeState !== "valid" && probeState !== "unverified") || anyProcessActive || queueBusy || pendingStart || runtimeBusy || !runtimeReady} onClick={() => beginDownload()}><Icon name="download" />{runtimeBusy ? "Встановлюємо компоненти…" : probeState === "checking" ? "Перевіряємо посилання…" : pendingStart ? "Починаємо…" : anyProcessActive || queueBusy ? "Інше завантаження триває" : "Завантажити"}</button>
          <p className="legal-note">Завантажуйте лише матеріали, на які маєте відповідні права.</p>
          </div>
        </section>}

        {activeView === "queue" && <QueueView
          queue={downloadQueue}
          input={queueInput}
          notice={queueNotice}
          runtimeReady={runtimeReady}
          singleDownloadActive={Boolean(active)}
          isWindows={isWindows}
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
          onNew={() => {
            if (queueHasActiveProcess(downloadQueueRef.current)) {
              setQueueNotice("Дочекайтеся завершення поточного файла або зупиніть його перед створенням нової черги.");
              return;
            }
            updateQueue(() => null);
            setQueueInput("");
            setQueueNotice("");
          }}
        />}

        {activeView === "history" && <HistoryView
          entries={historyEntries}
          retryingId={historyRetryPending}
          retryDisabled={Boolean(active || queueBusy)}
          onRetry={retryHistoryDownload}
          onClear={() => {
            if (!window.confirm("Очистити історію? Завантажені файли залишаться на диску.")) return;
            historyEntriesRef.current = [];
            setHistoryEntries([]);
            void invoke("clear_history_thumbnail_cache").catch(() => undefined);
          }}
        />}

      </main>

      {job && job.status !== "auth_required" && <div className="download-progress-backdrop" role="presentation">
        <section className={`job-card ${job.status}`} role="dialog" aria-modal="true" aria-labelledby="download-progress-title">
          {job.status === "completed" ? (
            <button type="button" className="job-icon job-icon-button" aria-label="Закрити повідомлення про завершення" onClick={() => setJob(null)}><Icon name="check"/></button>
          ) : job.status === "failed" ? (
            <button type="button" className="job-icon job-icon-button" aria-label="Закрити повідомлення про помилку" onClick={() => setJob(null)}><Icon name="x" size={24}/></button>
          ) : (
            <div className="job-icon" aria-label={`Етап ${jobStageNumber(job.status)}`}><span className="job-stage"><small>ЕТАП</small><strong>{jobStageNumber(job.status)}</strong></span></div>
          )}
          <div className="job-body">
            <div className="job-title"><div ref={jobTitleTextRef}><strong id="download-progress-title">{job.title}</strong><small>{job.status === "completed" ? "Готово" : job.status === "failed" ? job.message || "Помилка завантаження" : job.status === "cancelled" ? "Скасовано" : job.message}</small></div><span style={{ fontSize: `${jobValueFontSize}px` }}>{job.status === "postprocessing" || job.status === "converting" ? job.outputFormat : `${Math.round(job.percent)}%`}</span></div>
            <div className="job-source" title={job.url}><Icon name="link" size={12}/><span>{job.url}</span></div>
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
        </section>
      </div>}

      {cancelConfirmOpen && job && active && <div className="modal-backdrop" role="presentation">
        <div className="modal" role="dialog" aria-modal="true" aria-labelledby="cancel-title">
          <div className="modal-icon cancel"><Icon name="stop" size={28}/></div>
          <p className="eyebrow">АКТИВНЕ ЗАВАНТАЖЕННЯ</p>
          <h2 id="cancel-title">Скасувати процес?</h2>
          <p>Завантаження або конвертація зупиниться. Після підтвердження це вікно одразу закриється.</p>
          {cancelError && <div className="modal-detail cancel-error">{cancelError}</div>}
          <div className="modal-actions"><button className="secondary-button" disabled={cancelBusy} onClick={() => setCancelConfirmOpen(false)}>Продовжити</button><button className="cancel-confirm-button" disabled={cancelBusy} onClick={cancelJob}>{cancelBusy ? "Зупиняємо…" : "Так, скасувати"}</button></div>
        </div>
      </div>}

      {update && updatePromptOpen && !startupBusy && !active && !queueBusy && <div className="modal-backdrop update-modal-backdrop" role="presentation">
        <div className="modal update-modal" role="dialog" aria-modal="true" aria-labelledby="update-title">
          <div className="modal-icon update"><Icon name="refresh" size={30}/></div>
          <p className="eyebrow">ДОСТУПНЕ ОНОВЛЕННЯ</p>
          <h2 id="update-title">Нова версія {update.version}</h2>
          <p>Зараз встановлена версія <strong>{update.currentVersion}</strong>. Оновлення завантажиться, встановиться й автоматично перезапустить застосунок.</p>
          {update.body && <UpdateReleaseNotes notes={update.body}/>}
          {updateBusy && <div className="update-download-progress" role="progressbar" aria-label="Завантаження оновлення" aria-valuemin={0} aria-valuemax={100} aria-valuenow={updateProgress}><span style={{ width: `${Math.max(2, updateProgress)}%` }}/></div>}
          {updateInstallError && <div className="modal-detail cancel-error">{updateInstallError}</div>}
          <div className="modal-actions"><button className="secondary-button" disabled={updateBusy} onClick={() => setUpdatePromptOpen(false)}>Пізніше</button><button className="update-install-button" disabled={updateBusy} onClick={installAppUpdate}>{updateBusy ? `Оновлюємо ${updateProgress}%` : "Оновити зараз"}</button></div>
        </div>
      </div>}

      {vpnWarning && <div className="modal-backdrop" role="presentation">
        <div className="modal" role="dialog" aria-modal="true" aria-labelledby="vpn-title">
          <div className="modal-icon"><Icon name="shield" size={29}/></div>
          <p className="eyebrow">ПЕРЕВІРКА БЕЗПЕКИ</p>
          <h2 id="vpn-title">VPN не виявлено</h2>
          <p>Ви збираєтесь завантажити матеріал із <strong>{vpnWarning.host}</strong>. Не вдалося підтвердити, що VPN увімкнений.</p>
          <div className="modal-detail">{vpnWarning.status.detail}</div>
          <div className="modal-actions"><button className="secondary-button" onClick={dismissVpnWarning}>Скасувати</button><button className="warning-button" onClick={approveVpnWarning}>Продовжити без VPN</button></div>
        </div>
      </div>}

      {job?.status === "auth_required" && <div className="modal-backdrop" role="presentation">
        <div className="modal" role="dialog" aria-modal="true" aria-labelledby="auth-title">
          <div className="modal-icon auth"><Icon name="link" size={28}/></div>
          <p className="eyebrow">ПОТРІБНА АВТОРИЗАЦІЯ</p>
          <h2 id="auth-title">Увійдіть через браузер</h2>
          <p>Сайт просить підтвердити вік, акаунт або що ви не бот. Увійдіть на цей сайт у браузері та виберіть його нижче.</p>
          <label className="browser-picker">Ваш браузер<select value={cookieBrowser} onChange={(event) => setCookieBrowser(event.target.value)}>{!isWindows && <option value="safari">Safari</option>}<option value="edge">Microsoft Edge</option><option value="chrome">Google Chrome</option><option value="firefox">Firefox</option><option value="brave">Brave</option><option value="vivaldi">Vivaldi</option></select></label>
          <div className="modal-detail">yt-dlp прочитає cookies безпосередньо з обраного браузера лише для повторної спроби. yt-dlp BD не експортує їх у файл і не зберігає.</div>
          <div className="modal-actions"><button className="secondary-button" onClick={() => setJob(null)}>Скасувати</button><button className="warning-button auth" disabled={pendingStart} onClick={retryWithCookies}>{pendingStart ? "Перевіряємо…" : "Повторити з cookies"}</button></div>
        </div>
      </div>}

      {queueAlert && <div className="modal-backdrop queue-alert-backdrop" role="presentation">
        <div className="modal" role="dialog" aria-modal="true" aria-labelledby="queue-alert-title">
          <div className="modal-icon auth"><Icon name="list" size={28}/></div>
          <p className="eyebrow">ПАКЕТНЕ ЗАВАНТАЖЕННЯ</p>
          <h2 id="queue-alert-title">{queueAlert.title}</h2>
          <p>{queueAlert.message}</p>
          <div className="modal-actions"><button className="secondary-button" onClick={() => setQueueAlert(null)}>Закрити</button><button className="update-install-button" onClick={() => { setQueueAlert(null); setActiveView("queue"); }}>Відкрити список</button></div>
        </div>
      </div>}
    </div>
  );
}

function queueStatusLabel(item: QueueItem): string {
  if (item.status === "pending") return "Очікує";
  if (item.status === "starting") return "Підключення";
  if (item.status === "downloading") return `Завантаження ${Math.round(item.percent)}%`;
  if (item.status === "postprocessing") return "Об’єднання";
  if (item.status === "converting") return `Конвертація ${Math.round(item.percent)}%`;
  if (item.status === "completed") return "Готово";
  if (item.status === "failed") return "Помилка";
  if (item.status === "skipped") return "Пропущено";
  return "Перервано";
}

function queueStorageLabel(item: QueueItem): string | null {
  if (!item.storage) return null;
  const available = item.storage.availableSpace === null ? "вільне місце невідоме" : `вільно ${formatByteSize(item.storage.availableSpace)}`;
  return `${available} · потрібно до ${formatByteSize(item.storage.requiredSpace)}`;
}

function QueueView({ queue, input, notice, runtimeReady, singleDownloadActive, isWindows, onInput, onPaste, onAdd, onChangeItem, onRemoveItem, onSettings, onChooseFolder, onStart, onPause, onResume, onSkip, onStop, onRetry, onNew }: {
  queue: DownloadQueue | null;
  input: string;
  notice: string;
  runtimeReady: boolean;
  singleDownloadActive: boolean;
  isWindows: boolean;
  onInput: (value: string) => void;
  onPaste: (event: ClipboardEvent<HTMLInputElement | HTMLTextAreaElement>) => void;
  onAdd: () => void;
  onChangeItem: (id: string, value: string) => void;
  onRemoveItem: (id: string) => void;
  onSettings: (patch: Partial<QueueSettings>) => void;
  onChooseFolder: () => void;
  onStart: () => void;
  onPause: () => void;
  onResume: () => void;
  onSkip: () => void;
  onStop: () => void;
  onRetry: () => void;
  onNew: () => void;
}) {
  const progress = queueProgress(queue?.items || []);
  const activeItem = queue?.items.find((item) => item.id === queue.activeItemId) || null;
  const failedCount = queue?.items.filter((item) => item.status === "failed" || item.status === "interrupted").length || 0;
  const settingsEditable = !queue || queue.status === "draft" || (queue.status === "paused" && !queue.activeItemId) || (queue.status === "completed" && failedCount > 0);
  const settings = queue?.settings;
  const [settingsOpen, setSettingsOpen] = useState(false);

  return <section className="queue-panel">
    {settings && <details className="queue-settings-disclosure" open={settingsOpen} onToggle={(event) => setSettingsOpen(event.currentTarget.open)}>
      <summary>
        <span><small>Спільні для всієї черги</small><strong>Налаштування завантаження</strong></span>
        <span className="queue-settings-current">{settings.mode === "video" ? "Відео" : settings.audioFormat.toUpperCase()} · {settings.multiItem ? "Уся добірка" : "Один матеріал"}<Icon name="chevron" size={18}/></span>
      </summary>
      <div className="queue-settings">
        <div className="choice-grid">
          <div><span className="field-label">Що завантажуємо</span><div className="segmented"><button className={settings.mode === "video" ? "selected" : ""} disabled={!settingsEditable} onClick={() => onSettings({ mode: "video" })}>Відео</button><button className={settings.mode === "audio" ? "selected" : ""} disabled={!settingsEditable} onClick={() => onSettings({ mode: "audio" })}>Тільки аудіо</button></div></div>
          <div><label className="field-label" htmlFor="queue-quality">{settings.mode === "video" ? "Якість" : "Формат аудіо"}</label>{settings.mode === "video"
            ? <select id="queue-quality" disabled={!settingsEditable} value={settings.quality} onChange={(event) => onSettings({ quality: event.target.value })}><option value="best">Найкраща доступна (MP4)</option><option value="2160">До 4K</option><option value="1080">До 1080p</option><option value="720">До 720p</option><option value="480">До 480p</option></select>
            : <select id="queue-quality" disabled={!settingsEditable} value={settings.audioFormat} onChange={(event) => onSettings({ audioFormat: event.target.value })}><option value="mp3">MP3</option><option value="m4a">M4A</option><option value="opus">Opus</option><option value="wav">WAV</option></select>}</div>
        </div>
        <div className="queue-options">
          <label><input type="checkbox" disabled={!settingsEditable} checked={settings.subtitles} onChange={(event) => onSettings({ subtitles: event.target.checked })}/> Субтитри, якщо доступні</label>
          <label><input type="checkbox" disabled={!settingsEditable} checked={settings.multiItem} onChange={(event) => onSettings({ multiItem: event.target.checked })}/> Завантажувати всю добірку за кожним посиланням</label>
        </div>
        <label className="queue-browser-picker">Авторизація через браузер<select disabled={!settingsEditable} value={settings.cookiesBrowser || ""} onChange={(event) => onSettings({ cookiesBrowser: event.target.value || null })}><option value="">Не використовувати cookies</option>{!isWindows && <option value="safari">Safari</option>}<option value="edge">Microsoft Edge</option><option value="chrome">Google Chrome</option><option value="firefox">Firefox</option><option value="brave">Brave</option><option value="vivaldi">Vivaldi</option></select><small>Потрібно лише для сайтів, які просять увійти або підтвердити вік.</small></label>
        <div className="folder-row"><div className="folder-copy"><span className="field-label">Зберегти все у</span><div className="folder-value"><Icon name="folder" size={18}/><span>{settings.outputDir || "Папку не обрано"}</span></div></div><button className="secondary-button" disabled={!settingsEditable} onClick={onChooseFolder}>Змінити</button></div>
      </div>
    </details>}

    <div className={`queue-links-header ${queue?.status || "draft"}`}>
      <div>
        <small>Посилання для завантаження</small>
        <p>Кожне з нового рядка · список можна вставити цілком</p>
      </div>
      <span className="queue-links-count"><strong>{progress.total}</strong><small>із {QUEUE_LIMIT}</small></span>
      {queue?.status !== "draft" && <div className="queue-summary-progress"><span style={{ width: `${progress.percent}%` }}/></div>}
    </div>

    <div className="queue-items">
      {queue?.items.map((item, index) => {
        const itemEditable = canEditQueueItem(queue, item);
        const storageLabel = queueStorageLabel(item);
        return <article className={`queue-item ${item.status}`} key={item.id}>
        <span className="queue-item-index">{String(index + 1).padStart(2, "0")}</span>
        <div className="queue-item-copy">
          {itemEditable
            ? <input value={item.url} aria-label={`Посилання ${index + 1}`} onChange={(event) => onChangeItem(item.id, event.target.value)} />
            : <strong title={item.url}>{item.title || item.url}</strong>}
          <small className={normalizeHttpUrl(item.url) ? "" : "invalid"}>{queueStatusLabel(item)}{item.message && item.message !== queueStatusLabel(item) ? ` · ${item.message}` : ""}</small>
          {storageLabel && <small className={item.storage?.sufficient === false ? "invalid" : "queue-storage"}>{storageLabel}</small>}
          {item.id === activeItem?.id && <div className={`queue-item-progress ${item.status === "postprocessing" ? "indeterminate" : ""}`}><span style={item.status === "postprocessing" ? undefined : { width: `${item.percent}%` }}/></div>}
        </div>
        {item.status === "completed" ? <span className="queue-item-result">{formatByteSize(item.finalSize)}<Icon name="check" size={16}/></span>
          : itemEditable ? <button className="queue-remove" aria-label={`Видалити рядок ${index + 1}`} onClick={() => onRemoveItem(item.id)}><Icon name="x" size={16}/></button>
            : <span className={`queue-state ${item.status}`}>{queueStatusLabel(item)}</span>}
      </article>})}
      {settingsEditable && queue?.status !== "completed" && progress.total < QUEUE_LIMIT && <article className="queue-item queue-item-new">
        <span className="queue-item-index">{String(progress.total + 1).padStart(2, "0")}</span>
        <div className="queue-item-copy">
          <input id="queue-links" value={input} aria-label="Нове посилання" onChange={(event) => onInput(event.target.value)} onPaste={onPaste} onBlur={() => { if (input.trim()) onAdd(); }} onKeyDown={(event) => {
            if (event.key === "Enter") { event.preventDefault(); onAdd(); }
          }} placeholder={progress.total ? "Вставте наступне посилання…" : "Вставте сюди перше посилання…"} />
          <small>Вставте URL і натисніть Enter · можна вставити одразу кілька посилань</small>
        </div>
        <span className="queue-new-mark">+</span>
      </article>}
    </div>
    <p className="queue-limit">{progress.total} із {QUEUE_LIMIT}. Посилання перевірятимуться вже під час завантаження.</p>

    {notice && <div className="inline-error queue-notice">{notice}</div>}
    <div className="queue-actions">
      {(!queue || queue.status === "draft") && <button className="primary-button" disabled={!queue?.items.length || !runtimeReady || singleDownloadActive} onClick={onStart}><Icon name="download"/>Почати пакетне завантаження</button>}
      {queue?.status === "running" && <><button className="secondary-button" onClick={onPause}>Пауза після поточного</button><button className="secondary-button" disabled={!activeItem} onClick={onSkip}>Пропустити поточне</button><button className="cancel-confirm-button" onClick={onStop}>Зупинити все</button></>}
      {queue?.status === "paused" && <><button className="primary-button compact" disabled={singleDownloadActive || Boolean(activeItem)} onClick={onResume}><Icon name="download"/>{activeItem ? "Завершуємо поточний файл…" : "Продовжити"}</button>{activeItem && <button className="secondary-button" onClick={onSkip}>Пропустити поточне</button>}<button className="secondary-button" disabled={Boolean(activeItem)} onClick={onNew}>Нова черга</button></>}
      {queue?.status === "completed" && <>{failedCount > 0 && <button className="primary-button compact" onClick={onRetry}><Icon name="refresh"/>Спробувати проблемні ще раз ({failedCount})</button>}<button className="secondary-button" onClick={onNew}>Нова черга</button></>}
    </div>
  </section>;
}

function HistoryThumbnail({ entry }: { entry: HistoryEntry }) {
  const cachedSource = entry.cachedThumbnailPath ? convertFileSrc(entry.cachedThumbnailPath) : null;
  const [source, setSource] = useState(cachedSource || entry.thumbnail);

  useEffect(() => {
    setSource(cachedSource || entry.thumbnail);
  }, [cachedSource, entry.thumbnail]);

  return <div className="history-thumb">
    <span><Icon name="download" size={24}/></span>
    {source && <img src={source} alt="" onError={() => {
      if (source === cachedSource && entry.thumbnail) setSource(entry.thumbnail);
      else setSource(null);
    }} />}
  </div>;
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

function HistoryView({ entries, retryingId, retryDisabled, onRetry, onClear }: {
  entries: HistoryEntry[];
  retryingId: string | null;
  retryDisabled: boolean;
  onRetry: (entry: HistoryEntry) => void;
  onClear: () => void;
}) {
  const groups = groupHistoryEntries(entries);
  const [openGroups, setOpenGroups] = useState<Set<string>>(() => new Set(groups[0] ? [groups[0].key] : []));

  useEffect(() => {
    const firstKey = groups[0]?.key;
    if (!firstKey) return;
    setOpenGroups((current) => current.size ? current : new Set([firstKey]));
  }, [groups[0]?.key]);

  if (!entries.length) {
    return <section className="history-panel empty-history">
      <div className="empty-history-mark"><Icon name="clock" size={34}/></div>
      <p className="eyebrow">ПОКИ ПОРОЖНЬО</p>
      <h2>Завантаження з’являться тут</h2>
      <p>Після успішного завершення ми збережемо прев’ю, точний розмір, час і шлях до файла. Самі файли нікуди не копіюються.</p>
    </section>;
  }

  return <section className="history-panel">
    <div className="history-toolbar">
      <div><strong>{entries.length}</strong><span>{entries.length === 1 ? "файл в історії" : "файлів в історії"}</span></div>
      <button type="button" onClick={onClear}>Очистити історію</button>
    </div>
    <div className="history-groups">
      {groups.map((group) => <details
        key={group.key}
        open={openGroups.has(group.key)}
        onToggle={(event) => {
          const isOpen = event.currentTarget.open;
          setOpenGroups((current) => {
            if (current.has(group.key) === isOpen) return current;
            const next = new Set(current);
            if (isOpen) next.add(group.key); else next.delete(group.key);
            return next;
          });
        }}
      >
        <summary><span>{group.label}</span><small>{group.items.length}</small><Icon name="chevron" size={17}/></summary>
        <div className="history-list">
          {group.items.map((entry) => <article className={`history-item ${entry.available ? "available" : "missing"}`} key={entry.id}>
            <HistoryThumbnail entry={entry}/>
            <div className="history-copy">
              <div className="history-title-row"><strong>{entry.title}</strong><time dateTime={entry.downloadedAt}>{new Intl.DateTimeFormat("uk-UA", { hour: "2-digit", minute: "2-digit" }).format(new Date(entry.downloadedAt))}</time></div>
              <p>{[entry.uploader, entry.extractor, formatByteSize(entry.size)].filter(Boolean).join(" · ")}</p>
              <div className="history-location" title={entry.path}><span className={`history-state-dot ${entry.available ? "ready" : "missing"}`}/><span>{entry.available ? "Файл на місці" : "Файл недоступний"}</span><Icon name="folder" size={14}/><code>{entry.path}</code></div>
            </div>
            <div className="history-actions">
              <button type="button" className="history-retry-button" disabled={retryDisabled || retryingId !== null} onClick={() => onRetry(entry)}>{retryingId === entry.id ? "Готуємо…" : "Завантажити ще раз"}</button>
              <button type="button" disabled={!entry.available} onClick={() => revealItemInDir(entry.path)}>Показати</button>
              <button type="button" onClick={() => openUrl(entry.sourceUrl)}>Джерело</button>
            </div>
          </article>)}
        </div>
      </details>)}
    </div>
  </section>;
}

function RuntimeRow({ label, component, loading = false, retry }: { label: string; component?: ComponentStatus; loading?: boolean; retry?: () => void }) {
  const ready = component?.installed;
  return <div className="runtime-row"><span className={`component-dot ${loading ? "loading" : ready ? "ready" : "missing"}`}/><div><strong>{label}</strong><small>{loading ? "Завантажуємо й перевіряємо…" : ready ? shortVersion(component.version) : "Компонент недоступний"}</small></div>{ready && !loading ? <Icon name="check" size={15}/> : retry && !loading ? <button type="button" className="runtime-retry" aria-label={`Повторити встановлення ${label}`} onClick={retry}><Icon name="refresh" size={14}/></button> : null}</div>;
}

function StartupOverlay({ progress, message }: { progress: number; message: string }) {
  return <div className="startup-overlay" role="status" aria-live="polite">
    <div className="startup-body">
      <div className="startup-hands" aria-hidden="true">
        <img src={loadingHandOne} alt="" />
        <img src={loadingHandTwo} alt="" />
        <img src={loadingHandThree} alt="" />
      </div>
      <p className="eyebrow">YT-DLP BD</p>
      <h1>Запускаємо застосунок</h1>
      <p>{message}</p>
      <div className="startup-progress" aria-label="Прогрес запуску" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress)} role="progressbar"><span style={{ width: `${progress}%` }}/></div>
      <small>{Math.round(progress)}%</small>
    </div>
  </div>;
}

function RuntimePreparationOverlay({ stage, runtime, progress }: { stage: RuntimeStage; runtime: RuntimeStatus | null; progress: RuntimeInstallProgress | null }) {
  const components = [
    { key: "ytDlp", label: "yt-dlp", component: runtime?.ytDlp },
    { key: "ffmpeg", label: "ffmpeg", component: runtime?.ffmpeg },
    { key: "deno", label: "Deno", component: runtime?.deno },
  ] as const;

  return <div className="runtime-preparation-backdrop" role="dialog" aria-modal="true" aria-labelledby="runtime-preparation-title" aria-describedby="runtime-preparation-description">
    <div className="runtime-preparation-modal">
      <div className="runtime-preparation-spinner" aria-hidden="true"><span/></div>
      <p className="eyebrow">ПІДГОТОВКА ДО РОБОТИ</p>
      <h2 id="runtime-preparation-title">Готуємо компоненти</h2>
      <p id="runtime-preparation-description">Встановлюємо відсутні компоненти, необхідні для завантаження.</p>
      <div className="runtime-preparation-list" aria-label="Стан компонентів">
        {components.map(({ key, label, component }) => {
          const activeStage = stage === key;
          const ready = Boolean(component?.installed) && !activeStage;
          return <div className={activeStage ? "active" : ready ? "ready" : "waiting"} key={key}>
            <span className="runtime-preparation-status">{ready ? <Icon name="check" size={14}/> : <i/>}</span>
            <strong>{label}</strong>
            <small>{activeStage ? progress?.component === key && progress.total ? `${Math.min(100, Math.round((progress.downloaded / progress.total) * 100))}%` : "Встановлюємо…" : ready ? "Готово" : "Очікує"}</small>
          </div>;
        })}
      </div>
      <p className="runtime-wait-note">Будь ласка, зачекайте й не закривайте застосунок. Завантаження стане доступним автоматично.</p>
    </div>
  </div>;
}

export default App;
