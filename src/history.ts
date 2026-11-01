import { normalizeHttpUrl } from "./queue";

const HISTORY_STORAGE_KEY = "downloadHistory.v1";
export const HISTORY_LIMIT = 500;

export type HistoryEntry = {
  id: string;
  outcome?: "failed" | "cancelled" | "auth_required";
  message?: string;
  diagnostics?: string;
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

export function fileTitle(path: string): string {
  const filename = path.split(/[\\/]/).filter(Boolean).pop() || path;
  return filename.replace(/\.[^.]+$/, "") || filename;
}

export function parentDirectory(path: string): string {
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
  const outcome = ["failed", "cancelled", "auth_required"].includes(String(entry.outcome)) ? entry.outcome as HistoryEntry["outcome"] : undefined;
  const path = nonEmptyString(entry.path) || (outcome ? "" : null);
  const title = nonEmptyString(entry.title);
  const sourceUrl = nonEmptyString(entry.sourceUrl);
  const downloadedAt = nonEmptyString(entry.downloadedAt);
  if (!id || path === null || !title || !sourceUrl || !downloadedAt || !Number.isFinite(Date.parse(downloadedAt))) {
    return null;
  }
  const normalizedSourceUrl = normalizeHttpUrl(sourceUrl);
  if (!normalizedSourceUrl || !/^https?:/i.test(normalizedSourceUrl)) return null;
  const size = typeof entry.size === "number" && Number.isFinite(entry.size) && entry.size >= 0
    ? entry.size
    : 0;
  return {
    id,
    outcome,
    message: nonEmptyString(entry.message)?.slice(0, 1000) || undefined,
    sourceUrl: normalizedSourceUrl,
    title,
    thumbnail: nonEmptyString(entry.thumbnail),
    cachedThumbnailPath: nonEmptyString(entry.cachedThumbnailPath),
    uploader: nonEmptyString(entry.uploader),
    extractor: nonEmptyString(entry.extractor),
    size,
    downloadedAt,
    path,
    available: Boolean(path) && (typeof entry.available === "boolean" ? entry.available : true),
    settings: normalizeHistorySettings(entry.settings, path),
    ...(typeof entry.diagnostics === "string" ? { diagnostics: entry.diagnostics.slice(0, 4000) } : {}),
  };
}

export function limitHistoryDiagnostics(entries: HistoryEntry[]): HistoryEntry[] {
  return entries.map((entry, index) => {
    if (index < 20 || !entry.diagnostics) return entry;
    const { diagnostics: _diagnostics, ...rest } = entry;
    return rest;
  });
}

export function parseHistoryStorage(raw: string | null): HistoryEntry[] {
  try {
    const parsed: unknown = JSON.parse(raw || "[]");
    if (!Array.isArray(parsed)) return [];
    return limitHistoryDiagnostics(parsed.flatMap((entry) => {
      const normalized = normalizeHistoryEntry(entry);
      return normalized ? [normalized] : [];
    }).slice(0, HISTORY_LIMIT));
  } catch {
    return [];
  }
}

export function loadHistory(): HistoryEntry[] {
  try {
    return parseHistoryStorage(localStorage.getItem(HISTORY_STORAGE_KEY));
  } catch {
    return [];
  }
}

export function historyPaths(entries: HistoryEntry[]): string[] {
  return Array.from(new Set(entries.map((entry) => entry.path).filter(Boolean)));
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

export function shouldCacheHistoryThumbnail(kind: string, outputs: unknown[] | null, thumbnail: string | null): boolean {
  return ["completed", "failed", "cancelled"].includes(kind) && Boolean(outputs?.length) && Boolean(thumbnail);
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
