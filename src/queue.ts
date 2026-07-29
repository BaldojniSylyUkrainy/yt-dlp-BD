export const QUEUE_LIMIT = 50;
export const QUEUE_STORAGE_KEY = "downloadQueue.v1";

export type QueueItemStatus =
  | "pending"
  | "starting"
  | "downloading"
  | "postprocessing"
  | "converting"
  | "completed"
  | "failed"
  | "skipped"
  | "interrupted";

export type QueueStatus = "draft" | "running" | "paused" | "completed";

export type QueueOutput = {
  path: string;
  size: number;
};

export type QueueStorageEstimate = {
  requiredSpace: number;
  availableSpace: number | null;
  sufficient: boolean | null;
};

export type QueueSettings = {
  outputDir: string;
  mode: "video" | "audio";
  quality: string;
  audioFormat: string;
  subtitles: boolean;
  multiItem: boolean;
  cookiesBrowser: string | null;
};

export type QueueItem = {
  id: string;
  jobId: string | null;
  url: string;
  title: string;
  thumbnail: string | null;
  uploader: string | null;
  extractor: string | null;
  status: QueueItemStatus;
  percent: number;
  speed: string;
  eta: string;
  message: string;
  errorCode: string | null;
  outputs: QueueOutput[];
  finalSize: number;
  attempt: number;
  storage: QueueStorageEstimate | null;
};

export type DownloadQueue = {
  version: 1;
  status: QueueStatus;
  settings: QueueSettings;
  items: QueueItem[];
  activeItemId: string | null;
  createdAt: string;
  updatedAt: string;
};

const ITEM_STATUSES = new Set<QueueItemStatus>([
  "pending",
  "starting",
  "downloading",
  "postprocessing",
  "converting",
  "completed",
  "failed",
  "skipped",
  "interrupted",
]);

const QUEUE_STATUSES = new Set<QueueStatus>(["draft", "running", "paused", "completed"]);

export function normalizeHttpUrl(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed || trimmed.length > 8_192) return null;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed) && !/^https?:\/\//i.test(trimmed)) return null;
  try {
    const parsed = new URL(/^https?:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`);
    if (!(["http:", "https:"] as string[]).includes(parsed.protocol) || !parsed.hostname) return null;
    return parsed.toString();
  } catch {
    return null;
  }
}

export function queueUrlsFromText(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((entry) => entry.trim())
    .filter(Boolean);
}

export function createQueueItem(url: string, id: string = crypto.randomUUID()): QueueItem {
  return {
    id,
    jobId: null,
    url: url.trim(),
    title: "",
    thumbnail: null,
    uploader: null,
    extractor: null,
    status: "pending",
    percent: 0,
    speed: "—",
    eta: "—",
    message: "Очікує",
    errorCode: null,
    outputs: [],
    finalSize: 0,
    attempt: 0,
    storage: null,
  };
}

export function appendQueueUrls(
  current: QueueItem[],
  raw: string,
  idFactory: () => string = () => crypto.randomUUID(),
): { items: QueueItem[]; rejected: number } {
  const urls = queueUrlsFromText(raw);
  const capacity = Math.max(0, QUEUE_LIMIT - current.length);
  const accepted = urls.slice(0, capacity).map((url) => createQueueItem(url, idFactory()));
  return { items: [...current, ...accepted], rejected: Math.max(0, urls.length - accepted.length) };
}

export function commitQueueInput(
  current: DownloadQueue | null,
  fallbackSettings: QueueSettings,
  raw: string,
  now: string = new Date().toISOString(),
  idFactory: () => string = () => crypto.randomUUID(),
): { queue: DownloadQueue | null; rejected: number } {
  if (!raw.trim()) return { queue: current, rejected: 0 };
  const result = appendQueueUrls(current?.items || [], raw, idFactory);
  return {
    queue: {
      version: 1,
      status: current?.status === "completed" ? "draft" : current?.status || "draft",
      settings: current?.settings || fallbackSettings,
      items: result.items,
      activeItemId: current?.activeItemId || null,
      createdAt: current?.createdAt || now,
      updatedAt: now,
    },
    rejected: result.rejected,
  };
}

export function retryableQueueItems(items: QueueItem[]): QueueItem[] {
  return items.filter((item) => item.status === "failed" || item.status === "interrupted");
}

export function resetQueueItemsForRetry(items: QueueItem[]): QueueItem[] {
  return retryableQueueItems(items).map((item) => ({
    ...createQueueItem(item.url, item.id),
    title: item.title,
    thumbnail: item.thumbnail,
    uploader: item.uploader,
    extractor: item.extractor,
    attempt: item.attempt,
  }));
}

export function resetEntireQueueForReplay(items: QueueItem[]): QueueItem[] {
  return items.map((item) => ({
    ...createQueueItem(item.url, item.id),
    title: item.title,
    thumbnail: item.thumbnail,
    uploader: item.uploader,
    extractor: item.extractor,
    attempt: item.attempt,
  }));
}

export function nextPendingQueueItem(items: QueueItem[]): QueueItem | null {
  return items.find((item) => item.status === "pending" || item.status === "interrupted") || null;
}

export function queueProgress(items: QueueItem[]): { done: number; total: number; percent: number } {
  const done = items.filter((item) => ["completed", "failed", "skipped"].includes(item.status)).length;
  const activeFraction = items.reduce((sum, item) => {
    if (!["starting", "downloading", "postprocessing", "converting"].includes(item.status)) return sum;
    return sum + Math.max(0, Math.min(100, item.percent)) / 100;
  }, 0);
  return {
    done,
    total: items.length,
    percent: items.length ? Math.round(((done + activeFraction) / items.length) * 100) : 0,
  };
}

export function queueHasActiveProcess(queue: DownloadQueue | null): boolean {
  return Boolean(queue?.activeItemId);
}

export function queuePreventsOtherWork(queue: DownloadQueue | null): boolean {
  return Boolean(queue && (queue.status === "running" || queue.activeItemId));
}

export function canEditQueueItem(queue: DownloadQueue | null, item: QueueItem): boolean {
  if (!queue || queue.status === "draft") return true;
  if (queue.status === "paused" && !queue.activeItemId) return !["completed", "skipped"].includes(item.status);
  return queue.status === "completed" && ["failed", "interrupted"].includes(item.status);
}

function safeString(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value.slice(0, 8_192) : fallback;
}

function safeNullableString(value: unknown): string | null {
  return typeof value === "string" && value ? value.slice(0, 8_192) : null;
}

function normalizeQueueItem(value: unknown): QueueItem | null {
  if (!value || typeof value !== "object") return null;
  const item = value as Partial<QueueItem>;
  if (typeof item.id !== "string" || typeof item.url !== "string" || !item.url.trim()) return null;
  const status = ITEM_STATUSES.has(item.status as QueueItemStatus) ? item.status as QueueItemStatus : "pending";
  const restoredStatus = ["starting", "downloading", "postprocessing", "converting"].includes(status)
    ? "interrupted"
    : status;
  const outputs = Array.isArray(item.outputs)
    ? item.outputs.flatMap((output) => {
      if (!output || typeof output !== "object") return [];
      const candidate = output as Partial<QueueOutput>;
      return typeof candidate.path === "string" && typeof candidate.size === "number" && candidate.size >= 0
        ? [{ path: candidate.path, size: candidate.size }]
        : [];
    })
    : [];
  return {
    id: item.id,
    jobId: null,
    url: item.url.slice(0, 8_192),
    title: safeString(item.title),
    thumbnail: safeNullableString(item.thumbnail),
    uploader: safeNullableString(item.uploader),
    extractor: safeNullableString(item.extractor),
    status: restoredStatus,
    percent: Number.isFinite(item.percent) ? Math.min(100, Math.max(0, Number(item.percent))) : 0,
    speed: safeString(item.speed, "—"),
    eta: safeString(item.eta, "—"),
    message: restoredStatus === "interrupted" ? "Завантаження було перервано" : safeString(item.message, "Очікує"),
    errorCode: safeNullableString(item.errorCode),
    outputs,
    finalSize: Number.isFinite(item.finalSize) && Number(item.finalSize) >= 0 ? Number(item.finalSize) : 0,
    attempt: Number.isInteger(item.attempt) && Number(item.attempt) >= 0 ? Math.min(99, Number(item.attempt)) : 0,
    storage: item.storage && typeof item.storage === "object"
      ? {
          requiredSpace: Number.isFinite(item.storage.requiredSpace) && Number(item.storage.requiredSpace) >= 0 ? Number(item.storage.requiredSpace) : 0,
          availableSpace: Number.isFinite(item.storage.availableSpace) && Number(item.storage.availableSpace) >= 0 ? Number(item.storage.availableSpace) : null,
          sufficient: typeof item.storage.sufficient === "boolean" ? item.storage.sufficient : null,
        }
      : null,
  };
}

function normalizeQueueSettings(value: unknown, fallback: QueueSettings): QueueSettings {
  if (!value || typeof value !== "object") return fallback;
  const settings = value as Partial<QueueSettings>;
  return {
    outputDir: safeString(settings.outputDir, fallback.outputDir),
    mode: settings.mode === "audio" ? "audio" : "video",
    quality: safeString(settings.quality, fallback.quality),
    audioFormat: safeString(settings.audioFormat, fallback.audioFormat),
    subtitles: typeof settings.subtitles === "boolean" ? settings.subtitles : fallback.subtitles,
    multiItem: typeof settings.multiItem === "boolean" ? settings.multiItem : fallback.multiItem,
    cookiesBrowser: safeNullableString(settings.cookiesBrowser),
  };
}

export function parseQueueStorage(raw: string | null, fallbackSettings: QueueSettings): DownloadQueue | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as Partial<DownloadQueue>;
    if (!value || typeof value !== "object" || !Array.isArray(value.items)) return null;
    const items = value.items.slice(0, QUEUE_LIMIT).flatMap((item) => {
      const normalized = normalizeQueueItem(item);
      return normalized ? [normalized] : [];
    });
    const restoredFromActive = items.some((item) => item.status === "interrupted");
    const status = QUEUE_STATUSES.has(value.status as QueueStatus) ? value.status as QueueStatus : "draft";
    return {
      version: 1,
      status: restoredFromActive || status === "running" ? "paused" : status,
      settings: normalizeQueueSettings(value.settings, fallbackSettings),
      items,
      activeItemId: null,
      createdAt: safeString(value.createdAt, new Date().toISOString()),
      updatedAt: safeString(value.updatedAt, new Date().toISOString()),
    };
  } catch {
    return null;
  }
}
