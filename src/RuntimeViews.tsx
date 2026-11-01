import { DialogSurface } from "./DialogSurface";
import { Icon } from "./Icon";
import loadingHandOne from "./assets/logo-call-me-hand-frame-1.png";
import loadingHandTwo from "./assets/logo-call-me-hand-frame-2.png";
import loadingHandThree from "./assets/logo-call-me-hand-frame-3.png";

export type ComponentStatus = {
  installed: boolean;
  version: string | null;
  path: string | null;
  managed: boolean;
};

export type RuntimeStatus = {
  ytDlp: ComponentStatus;
  ffmpeg: ComponentStatus;
  deno: ComponentStatus;
  runtimeDir: string;
  platform: string;
};

export type RuntimeStage = "ytDlp" | "ffmpeg" | "deno" | null;

export type RuntimeInstallProgress = {
  component: Exclude<RuntimeStage, null>;
  downloaded: number;
  total: number | null;
};

export function shouldShowRuntimeRetry(loading: boolean, hasRetry: boolean): boolean {
  return !loading && hasRetry;
}

function shortVersion(value: string | null): string {
  if (!value) return "Не встановлено";
  return value.replace(/^nightly@/, "").split("\n")[0];
}

export function RuntimeRow({ label, component, loading = false, retry }: { label: string; component?: ComponentStatus; loading?: boolean; retry?: () => void }) {
  const ready = component?.installed;
  const showRetry = shouldShowRuntimeRetry(loading, Boolean(retry));
  return <div className="runtime-row"><span className={`component-dot ${loading ? "loading" : ready ? "ready" : "missing"}`}/><div><strong>{label}</strong><small>{loading ? "Завантажуємо й перевіряємо…" : ready ? shortVersion(component.version) : "Компонент недоступний"}</small></div>{showRetry ? <button type="button" className="runtime-retry" aria-label={`Повторити встановлення ${label}`} onClick={retry}><Icon name="refresh" size={14}/></button> : ready && !loading ? <Icon name="check" size={15}/> : null}</div>;
}

export function StartupOverlay({ progress, message }: { progress: number; message: string }) {
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

export function RuntimePreparationOverlay({ stage, runtime, progress }: { stage: RuntimeStage; runtime: RuntimeStatus | null; progress: RuntimeInstallProgress | null }) {
  const components = [
    { key: "ytDlp", label: "yt-dlp", component: runtime?.ytDlp },
    { key: "ffmpeg", label: "ffmpeg", component: runtime?.ffmpeg },
    { key: "deno", label: "Deno", component: runtime?.deno },
  ] as const;

  return <div className="runtime-preparation-backdrop" role="presentation">
    <DialogSurface className="runtime-preparation-modal" labelledBy="runtime-preparation-title" describedBy="runtime-preparation-description">
      <div className="runtime-preparation-spinner" aria-hidden="true"><span/></div>
      <p className="eyebrow">ПІДГОТОВКА ДО РОБОТИ</p>
      <h2 id="runtime-preparation-title">Готуємо компоненти</h2>
      <p id="runtime-preparation-description">Перевіряємо та за потреби оновлюємо компоненти, необхідні для завантаження.</p>
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
    </DialogSurface>
  </div>;
}
