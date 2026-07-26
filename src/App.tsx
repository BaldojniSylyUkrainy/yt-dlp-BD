import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { downloadDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import brandLogo from "./assets/logo-call-me-hands.png";
import loadingHandOne from "./assets/logo-call-me-hand-frame-1.png";
import loadingHandTwo from "./assets/logo-call-me-hand-frame-2.png";
import loadingHandThree from "./assets/logo-call-me-hand-frame-3.png";
import "./App.css";

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
};

type VpnStatus = {
  detected: boolean;
  interfaceName: string | null;
  confidence: string;
  detail: string;
};

type DownloadEvent = {
  id: string;
  kind: "progress" | "postprocess" | "conversion_progress" | "storage_estimate" | "retrying" | "log" | "completed" | "failed" | "cancelled" | "auth_required";
  percent: number | null;
  speed: string | null;
  eta: string | null;
  message: string | null;
  storage: StorageEstimate | null;
};

type StorageEstimate = {
  estimatedSize: number;
  requiredSpace: number;
  availableSpace: number | null;
  sufficient: boolean | null;
};

type Job = {
  id: string;
  url: string;
  title: string;
  status: "starting" | "downloading" | "postprocessing" | "converting" | "completed" | "failed" | "cancelled" | "auth_required";
  percent: number;
  speed: string;
  eta: string;
  message: string;
  storage: StorageEstimate | null;
};

type RuntimeStage = "ytDlp" | "ffmpeg" | "deno" | null;

type MediaPreview = {
  title: string;
  thumbnail: string | null;
  duration: number | null;
  uploader: string | null;
  extractor: string | null;
  webpageUrl: string | null;
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

function isRussianDomain(host: string): boolean {
  return host === "ru" || host.endsWith(".ru") || host === "xn--p1ai" || host.endsWith(".xn--p1ai");
}

function isLikelyMultiItemUrl(value: string): boolean {
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
  const mainContentRef = useRef<HTMLElement>(null);
  const jobTitleTextRef = useRef<HTMLDivElement>(null);
  const pendingStorageEstimates = useRef(new Map<string, StorageEstimate>());
  const [runtime, setRuntime] = useState<RuntimeStatus | null>(null);
  const [runtimeBusy, setRuntimeBusy] = useState(false);
  const [runtimeStage, setRuntimeStage] = useState<RuntimeStage>(null);
  const [runtimeError, setRuntimeError] = useState("");
  const [url, setUrl] = useState("");
  const [outputDir, setOutputDir] = useState(localStorage.getItem("outputDir") || "");
  const [outputFreeSpace, setOutputFreeSpace] = useState<number | null>(null);
  const [mode, setMode] = useState<"video" | "audio">("video");
  const [quality, setQuality] = useState("best");
  const [audioFormat, setAudioFormat] = useState("mp3");
  const [subtitles, setSubtitles] = useState(false);
  const [job, setJob] = useState<Job | null>(null);
  const [formError, setFormError] = useState("");
  const [vpnWarning, setVpnWarning] = useState<{ host: string; status: VpnStatus } | null>(null);
  const [pendingStart, setPendingStart] = useState(false);
  const [update, setUpdate] = useState<Update | null>(null);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateProgress, setUpdateProgress] = useState(0);
  const [cookieBrowser, setCookieBrowser] = useState(localStorage.getItem("cookieBrowser") || "safari");
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

  const active = job && ["starting", "downloading", "postprocessing", "converting"].includes(job.status);
  const runtimeReady = Boolean(runtime?.ytDlp.installed && runtime?.ffmpeg.installed && runtime?.deno.installed);
  const quickThumbnail = youtubeThumbnailFromInput(url);

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
  }, []);

  const refreshManagedComponents = useCallback(async () => {
    const now = Date.now();
    const schedules = [
      { command: "update_ytdlp", key: "runtimeUpdate.ytDlp", interval: 24 * 60 * 60 * 1000 },
      { command: "install_ffmpeg", key: "runtimeUpdate.ffmpeg", interval: 7 * 24 * 60 * 60 * 1000 },
      { command: "install_deno", key: "runtimeUpdate.deno", interval: 7 * 24 * 60 * 60 * 1000 },
    ];
    const due = schedules.filter(({ key, interval }) => now - Number(localStorage.getItem(key) || 0) >= interval);
    if (!due.length) return;
    due.forEach(({ key }) => localStorage.setItem(key, String(now)));
    await Promise.allSettled(due.map(({ command }) => invoke<void>(command)));
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
        } catch (error) {
          issues.push(`${component.label}: ${String(error)}`);
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

  useEffect(() => {
    downloadDir().then((directory) => {
      setOutputDir((current) => {
        if (current) return current;
        localStorage.setItem("outputDir", directory);
        return directory;
      });
    }).catch(() => undefined);
    if (!maintenanceStarted.current) {
      maintenanceStarted.current = true;
      maintainRuntime();
    }
    check({ timeout: 8_000 }).then(setUpdate).catch(() => undefined);
  }, [maintainRuntime]);

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
    if (!parsedHost) {
      setProbeState("invalid");
      setProbeError("Це не схоже на посилання");
      return;
    }
    setProbeState("checking");
    let softDeadline = 0;
    const timer = window.setTimeout(async () => {
      softDeadline = window.setTimeout(() => {
        if (probeSequence.current !== sequence) return;
        setProbeError("Швидка перевірка займає більше часу, ніж очікувалось");
        setProbeState("unverified");
      }, 4_000);
      try {
        const normalized = /^https?:\/\//i.test(value) ? value : `https://${value}`;
        const result = await invoke<MediaPreview>("probe_url", { url: normalized });
        if (probeSequence.current !== sequence) return;
        setPreview(result);
        setProbeState("valid");
      } catch (error) {
        if (probeSequence.current !== sequence) return;
        setProbeError(String(error));
        setProbeState("unverified");
      } finally {
        window.clearTimeout(softDeadline);
      }
    }, 140);
    return () => {
      window.clearTimeout(timer);
      window.clearTimeout(softDeadline);
    };
  }, [url, runtime?.ytDlp.installed]);

  useEffect(() => {
    const unlisten = listen<DownloadEvent>("download-event", ({ payload }) => {
      if (payload.kind === "storage_estimate" && payload.storage) {
        pendingStorageEstimates.current.set(payload.id, payload.storage);
        if (payload.storage.availableSpace !== null) setOutputFreeSpace(payload.storage.availableSpace);
      }
      setJob((current) => {
        if (!current || current.id !== payload.id) return current;
        if (payload.kind === "storage_estimate") {
          pendingStorageEstimates.current.delete(payload.id);
          return { ...current, storage: payload.storage || current.storage };
        }
        if (payload.kind === "cancelled") return null;
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
            title: payload.message || current.title,
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
        if (payload.kind === "log") return { ...current, message: payload.message || current.message };
        return {
          ...current,
          status: payload.kind,
          percent: payload.kind === "completed" ? 100 : current.percent,
          message: payload.kind === "auth_required" ? "Потрібен вхід через браузер" : payload.message || current.message,
        };
      });
    });
    return () => { unlisten.then((dispose) => dispose()); };
  }, []);

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

  async function launchDownload(downloadUrl: string, parsedHost: string, cookiesBrowser?: string) {
    setPendingStart(true);
    try {
      const id = await invoke<string>("start_download", {
        request: {
          url: /^https?:\/\//i.test(downloadUrl.trim()) ? downloadUrl.trim() : `https://${downloadUrl.trim()}`,
          outputDir,
          mode,
          quality,
          audioFormat,
          subtitles,
          multiItem: isLikelyMultiItemUrl(downloadUrl),
          cookiesBrowser: cookiesBrowser || null,
        },
      });
      const storage = pendingStorageEstimates.current.get(id) || null;
      pendingStorageEstimates.current.delete(id);
      setJob({ id, url: downloadUrl, title: parsedHost, status: "starting", percent: 0, speed: "—", eta: "—", message: cookiesBrowser ? `Вхід через ${cookiesBrowser}…` : "Підключення…", storage });
      setUrl("");
    } catch (error) {
      setFormError(String(error));
    } finally {
      setPendingStart(false);
    }
  }

  async function beginDownload(skipVpnCheck = false) {
    setFormError("");
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

    if (!skipVpnCheck && isRussianDomain(parsedHost)) {
      setPendingStart(true);
      try {
        const vpn = await invoke<VpnStatus>("check_vpn", { host: parsedHost });
        if (!vpn.detected) {
          setVpnWarning({ host: parsedHost, status: vpn });
          setPendingStart(false);
          return;
        }
      } catch {
        setVpnWarning({
          host: parsedHost,
          status: { detected: false, interfaceName: null, confidence: "unknown", detail: "Не вдалося перевірити стан VPN" },
        });
        setPendingStart(false);
        return;
      }
    }

    await launchDownload(url, parsedHost);
  }

  async function retryWithCookies() {
    if (!job) return;
    const parsedHost = hostFromInput(job.url);
    if (!parsedHost) return;
    localStorage.setItem("cookieBrowser", cookieBrowser);
    await launchDownload(job.url, parsedHost, cookieBrowser);
  }

  async function cancelJob() {
    if (!job) return;
    const jobId = job.id;
    setCancelBusy(true);
    setCancelError("");
    try {
      await invoke("cancel_download", { id: jobId });
      setCancelConfirmOpen(false);
      setJob((current) => current?.id === jobId ? null : current);
    } catch (error) {
      setCancelError(String(error));
    } finally {
      setCancelBusy(false);
    }
  }

  async function installAppUpdate() {
    if (!update) return;
    setUpdateBusy(true);
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
      setFormError(`Не вдалося встановити оновлення: ${String(error)}`);
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
          <button className="nav-item active"><span className="nav-index">01</span>Завантажити<Icon name="chevron" /></button>
          <button className="nav-item"><span className="nav-index">02</span>Історія<span className="soon">згодом</span></button>
          <button className="nav-item"><span className="nav-index">03</span>Налаштування<span className="soon">згодом</span></button>
        </nav>
        <div className="runtime-panel">
          <div className="runtime-heading"><span>Компоненти</span><small>{runtimeBusy ? "перевірка" : runtimeReady ? "готові" : "очікуємо"}</small></div>
          <RuntimeRow label="yt-dlp" component={runtime?.ytDlp} loading={runtimeStage === "ytDlp"} />
          <RuntimeRow label="ffmpeg" component={runtime?.ffmpeg} loading={runtimeStage === "ffmpeg"} />
          <RuntimeRow label="Deno" component={runtime?.deno} loading={runtimeStage === "deno"} />
          {runtimeError && <p className="runtime-error">{runtimeError}</p>}
        </div>
        <div className="support-card">
          <span>Щось не працює?</span>
          <button onClick={contactSupport}>baldojnisyly@gmail.com <span aria-hidden="true">↗</span></button>
        </div>
        <div className="sidebar-footer"><span className="status-dot"/>Версія 0.1.0 · Apple Silicon</div>
      </aside>

      <main ref={mainContentRef} className={`main-content ${mainScrollable ? "is-scrollable" : "is-fixed"}`}>
        <header className="topbar">
          <div><p className="eyebrow">ІНСТРУМЕНТ / 01</p><h1>Завантажити</h1><p>Одне посилання. Відео, аудіо, субтитри.</p></div>
          {update && <button className="update-pill" disabled={updateBusy} onClick={installAppUpdate}><Icon name="refresh" size={16}/>{updateBusy ? `Оновлення ${updateProgress}%` : `Доступна v${update.version}`}</button>}
        </header>

        <section className="download-card" aria-busy={runtimeBusy}>
          {runtimeBusy && <RuntimePreparationOverlay stage={runtimeStage} runtime={runtime} />}
          <div className="download-card-content" inert={runtimeBusy ? true : undefined}>
          <label className="field-label" htmlFor="media-url">Посилання</label>
          <div className={`url-field ${probeState}`}>
            <Icon name="link" />
            <input id="media-url" value={url} onChange={(event) => setUrl(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !active) beginDownload(); }} placeholder="https://youtube.com/watch?v=…" autoFocus />
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
            <div><strong>{preview.title}</strong><p>{[preview.uploader, formatDuration(preview.duration), preview.extractor].filter(Boolean).join(" · ")}</p><small><Icon name="check" size={13}/>yt-dlp підтвердив, що відео можна завантажити</small></div>
          </div>}
          {probeState === "unverified" && <div className="url-warning" title={probeError}><span aria-hidden="true">!</span><span>Не вдалося швидко підтвердити посилання. yt-dlp все одно може спробувати його завантажити.</span></div>}
          {probeState === "invalid" && <div className="url-error"><Icon name="x" size={14}/><span>{probeError || "yt-dlp не може завантажити це посилання"}</span></div>}

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
          <button className="primary-button" disabled={(probeState !== "valid" && probeState !== "unverified") || !!active || pendingStart || runtimeBusy || !runtimeReady} onClick={() => beginDownload()}><Icon name="download" />{runtimeBusy ? "Встановлюємо компоненти…" : probeState === "checking" ? "Перевіряємо посилання…" : pendingStart ? "Перевіряємо…" : active ? "Завантаження триває" : probeState === "unverified" ? "Спробувати завантажити" : "Завантажити"}</button>
          <p className="legal-note">Завантажуйте лише матеріали, на які маєте відповідні права.</p>
          </div>
        </section>

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
            <div className="job-title"><div ref={jobTitleTextRef}><strong id="download-progress-title">{job.title}</strong><small>{job.status === "completed" ? "Готово" : job.status === "failed" ? job.message || "Помилка завантаження" : job.status === "cancelled" ? "Скасовано" : job.message}</small></div><span style={{ fontSize: `${jobValueFontSize}px` }}>{job.status === "postprocessing" || job.status === "converting" ? "MP4" : `${Math.round(job.percent)}%`}</span></div>
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

      {vpnWarning && <div className="modal-backdrop" role="presentation">
        <div className="modal" role="dialog" aria-modal="true" aria-labelledby="vpn-title">
          <div className="modal-icon"><Icon name="shield" size={29}/></div>
          <p className="eyebrow">ПЕРЕВІРКА БЕЗПЕКИ</p>
          <h2 id="vpn-title">VPN не виявлено</h2>
          <p>Ви збираєтесь завантажити матеріал із <strong>{vpnWarning.host}</strong>. Не вдалося підтвердити, що VPN увімкнений.</p>
          <div className="modal-detail">{vpnWarning.status.detail}</div>
          <div className="modal-actions"><button className="secondary-button" onClick={() => setVpnWarning(null)}>Скасувати</button><button className="warning-button" onClick={() => { setVpnWarning(null); beginDownload(true); }}>Завантажити все одно</button></div>
        </div>
      </div>}

      {job?.status === "auth_required" && <div className="modal-backdrop" role="presentation">
        <div className="modal" role="dialog" aria-modal="true" aria-labelledby="auth-title">
          <div className="modal-icon auth"><Icon name="link" size={28}/></div>
          <p className="eyebrow">ПОТРІБНА АВТОРИЗАЦІЯ</p>
          <h2 id="auth-title">Увійдіть через браузер</h2>
          <p>Сайт просить підтвердити вік, акаунт або що ви не бот. Увійдіть на цей сайт у браузері та виберіть його нижче.</p>
          <label className="browser-picker">Ваш браузер<select value={cookieBrowser} onChange={(event) => setCookieBrowser(event.target.value)}><option value="safari">Safari</option><option value="chrome">Google Chrome</option><option value="firefox">Firefox</option><option value="brave">Brave</option><option value="edge">Microsoft Edge</option><option value="vivaldi">Vivaldi</option></select></label>
          <div className="modal-detail">yt-dlp прочитає cookies безпосередньо з обраного браузера лише для повторної спроби. yt-dlp BD не експортує їх у файл і не зберігає.</div>
          <div className="modal-actions"><button className="secondary-button" onClick={() => setJob(null)}>Скасувати</button><button className="warning-button auth" disabled={pendingStart} onClick={retryWithCookies}>{pendingStart ? "Перевіряємо…" : "Повторити з cookies"}</button></div>
        </div>
      </div>}
    </div>
  );
}

function RuntimeRow({ label, component, loading = false }: { label: string; component?: ComponentStatus; loading?: boolean }) {
  const ready = component?.installed;
  return <div className="runtime-row"><span className={`component-dot ${loading ? "loading" : ready ? "ready" : "missing"}`}/><div><strong>{label}</strong><small>{loading ? "Перевіряємо й оновлюємо…" : ready ? shortVersion(component.version) : "Готуємо автоматично"}</small></div>{ready && !loading && <Icon name="check" size={15}/>}</div>;
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

function RuntimePreparationOverlay({ stage, runtime }: { stage: RuntimeStage; runtime: RuntimeStatus | null }) {
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
            <small>{activeStage ? "Встановлюємо…" : ready ? "Готово" : "Очікує"}</small>
          </div>;
        })}
      </div>
      <p className="runtime-wait-note">Будь ласка, зачекайте й не закривайте застосунок. Завантаження стане доступним автоматично.</p>
    </div>
  </div>;
}

export default App;
