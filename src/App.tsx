import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { downloadDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import brandLogo from "./assets/logo.png";
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
  kind: "progress" | "log" | "completed" | "failed" | "cancelled" | "auth_required";
  percent: number | null;
  speed: string | null;
  eta: string | null;
  message: string | null;
};

type Job = {
  id: string;
  url: string;
  title: string;
  status: "starting" | "downloading" | "completed" | "failed" | "cancelled" | "auth_required";
  percent: number;
  speed: string;
  eta: string;
  message: string;
};

type RuntimeStage = "ytDlp" | "ffmpeg" | "deno" | null;

type IconName =
  | "download"
  | "clock"
  | "settings"
  | "folder"
  | "link"
  | "shield"
  | "check"
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

function shortVersion(value: string | null): string {
  if (!value) return "Не встановлено";
  return value.replace(/^nightly@/, "").split("\n")[0];
}

function App() {
  const maintenanceStarted = useRef(false);
  const [runtime, setRuntime] = useState<RuntimeStatus | null>(null);
  const [runtimeBusy, setRuntimeBusy] = useState(false);
  const [runtimeStage, setRuntimeStage] = useState<RuntimeStage>(null);
  const [runtimeError, setRuntimeError] = useState("");
  const [url, setUrl] = useState("");
  const [outputDir, setOutputDir] = useState(localStorage.getItem("outputDir") || "");
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

  const active = job && ["starting", "downloading"].includes(job.status);
  const host = useMemo(() => hostFromInput(url), [url]);
  const runtimeReady = Boolean(runtime?.ytDlp.installed && runtime?.ffmpeg.installed && runtime?.deno.installed);

  const maintainRuntime = useCallback(async () => {
    setRuntimeBusy(true);
    setRuntimeError("");
    const issues: string[] = [];
    try {
      let status = await invoke<RuntimeStatus>("runtime_status");
      setRuntime(status);

      setRuntimeStage("ytDlp");
      try {
        status = await invoke<RuntimeStatus>(status.ytDlp.installed ? "update_ytdlp" : "install_ytdlp");
        setRuntime(status);
      } catch (error) {
        issues.push(`yt-dlp: ${String(error)}`);
      }

      setRuntimeStage("ffmpeg");
      try {
        status = await invoke<RuntimeStatus>("install_ffmpeg");
        setRuntime(status);
      } catch (error) {
        issues.push(`ffmpeg: ${String(error)}`);
      }

      setRuntimeStage("deno");
      try {
        status = await invoke<RuntimeStatus>("install_deno");
        setRuntime(status);
      } catch (error) {
        issues.push(`Deno: ${String(error)}`);
      }

      setRuntimeError(issues.join(" · "));
    } catch (error) {
      setRuntimeError(String(error));
    } finally {
      setRuntimeStage(null);
      setRuntimeBusy(false);
    }
  }, []);

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
    const unlisten = listen<DownloadEvent>("download-event", ({ payload }) => {
      setJob((current) => {
        if (!current || current.id !== payload.id) return current;
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
        if (payload.kind === "log") return { ...current, message: payload.message || current.message };
        return {
          ...current,
          status: payload.kind,
          percent: payload.kind === "completed" ? 100 : current.percent,
          message: payload.kind === "auth_required" ? "Потрібен вхід через браузер" : current.message,
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
          cookiesBrowser: cookiesBrowser || null,
        },
      });
      setJob({ id, url: downloadUrl, title: parsedHost, status: "starting", percent: 0, speed: "—", eta: "—", message: cookiesBrowser ? `Вхід через ${cookiesBrowser}…` : "Підключення…" });
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
    try {
      await invoke("cancel_download", { id: job.id });
    } catch (error) {
      setFormError(String(error));
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
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark"><img src={brandLogo} alt="" /></div>
          <div><strong>yt-dlp BD</strong><small>Baldojnyi Downloader</small></div>
        </div>
        <nav>
          <button className="nav-item active"><Icon name="download" />Завантажити</button>
          <button className="nav-item"><Icon name="clock" />Історія<span className="soon">згодом</span></button>
          <button className="nav-item"><Icon name="settings" />Налаштування<span className="soon">згодом</span></button>
        </nav>
        <div className="runtime-panel">
          <div className="runtime-heading"><span>Компоненти</span><small>{runtimeBusy ? "перевірка" : runtimeReady ? "готові" : "очікуємо"}</small></div>
          <RuntimeRow label="yt-dlp" component={runtime?.ytDlp} loading={runtimeStage === "ytDlp"} />
          <RuntimeRow label="ffmpeg" component={runtime?.ffmpeg} loading={runtimeStage === "ffmpeg"} />
          <RuntimeRow label="Deno" component={runtime?.deno} loading={runtimeStage === "deno"} />
          {runtimeError && <p className="runtime-error">{runtimeError}</p>}
        </div>
        <div className="sidebar-footer"><span className="status-dot"/>Версія 0.1.0 · Apple Silicon</div>
      </aside>

      <main className="main-content">
        <header className="topbar">
          <div><p className="eyebrow">НОВЕ ЗАВАНТАЖЕННЯ</p><h1>Збережіть те, що хочете</h1><p>Вставте посилання — усе інше зробить yt-dlp BD.</p></div>
          {update && <button className="update-pill" disabled={updateBusy} onClick={installAppUpdate}><Icon name="refresh" size={16}/>{updateBusy ? `Оновлення ${updateProgress}%` : `Доступна v${update.version}`}</button>}
          <img className="hero-mark" src={brandLogo} alt="" aria-hidden="true" />
        </header>

        <section className="download-card">
          <div className="folk-thread" aria-hidden="true"><span/><span/><i/><span/><span/></div>
          <label className="field-label" htmlFor="media-url">Посилання</label>
          <div className={`url-field ${host ? "valid" : ""}`}>
            <Icon name="link" />
            <input id="media-url" value={url} onChange={(event) => setUrl(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !active) beginDownload(); }} placeholder="https://youtube.com/watch?v=…" autoFocus />
            {host && <span className="valid-check"><Icon name="check" size={15}/></span>}
          </div>

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
                <select id="quality" value={quality} onChange={(event) => setQuality(event.target.value)}><option value="best">Найкраща доступна</option><option value="2160">До 4K</option><option value="1080">До 1080p</option><option value="720">До 720p</option><option value="480">До 480p</option></select>
              ) : (
                <select id="quality" value={audioFormat} onChange={(event) => setAudioFormat(event.target.value)}><option value="mp3">MP3</option><option value="m4a">M4A</option><option value="opus">Opus</option><option value="wav">WAV</option></select>
              )}
            </div>
          </div>

          <label className="toggle-row"><span><strong>Завантажити субтитри</strong><small>Українські та англійські, якщо доступні</small></span><input type="checkbox" checked={subtitles} onChange={(event) => setSubtitles(event.target.checked)}/><i/></label>

          <div className="folder-row">
            <div><span className="field-label">Зберегти у</span><div className="folder-value"><Icon name="folder" size={18}/><span>{outputDir || "Папку не обрано"}</span></div></div>
            <button className="secondary-button" onClick={chooseFolder}>Змінити</button>
          </div>

          {formError && <div className="inline-error">{formError}</div>}
          {runtimeBusy && <div className="runtime-notice"><span/><div><strong>Готуємо компоненти</strong><small>Перевіряємо {runtimeStage === "ytDlp" ? "yt-dlp" : runtimeStage === "ffmpeg" ? "ffmpeg" : runtimeStage === "deno" ? "Deno" : "середовище"}. Завантаження стане доступним автоматично.</small></div></div>}
          <button className="primary-button" disabled={!url.trim() || !!active || pendingStart || runtimeBusy || !runtimeReady} onClick={() => beginDownload()}><Icon name="download" />{runtimeBusy ? "Оновлюємо компоненти…" : pendingStart ? "Перевіряємо…" : active ? "Завантаження триває" : "Завантажити"}</button>
          <p className="legal-note">Завантажуйте лише матеріали, на які маєте відповідні права.</p>
        </section>

        {job && <section className={`job-card ${job.status}`}>
          <div className="job-icon"><Icon name={job.status === "completed" ? "check" : "download"}/></div>
          <div className="job-body">
            <div className="job-title"><div><strong>{job.title}</strong><small>{job.status === "completed" ? "Готово" : job.status === "failed" ? "Помилка завантаження" : job.status === "cancelled" ? "Скасовано" : job.message}</small></div><span>{Math.round(job.percent)}%</span></div>
            <div className="progress-track"><span style={{ width: `${job.percent}%` }}/></div>
            <div className="job-meta"><span>{job.speed}</span><span>{job.eta !== "—" ? `Залишилось ${job.eta}` : ""}</span></div>
          </div>
          {active && <button className="stop-button" aria-label="Зупинити" onClick={cancelJob}><Icon name="stop" size={18}/></button>}
        </section>}
      </main>

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

export default App;
