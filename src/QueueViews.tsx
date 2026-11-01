import { useState, type ClipboardEvent } from "react";
import { DialogSurface } from "./DialogSurface";
import { formatByteSize } from "./format";
import { Icon } from "./Icon";
import { canOfferBrowserDiscovery, DownloadTools } from "./MediaTools";
import { canEditQueueItem, normalizeHttpUrl, QUEUE_LIMIT, queueProgress, type DownloadQueue, type QueueItem, type QueueSettings } from "./queue";

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

export function QueueView({ queue, input, notice, runtimeReady, singleDownloadActive, onInput, onPaste, onAdd, onChangeItem, onRemoveItem, onSettings, onChooseFolder, onStart, onPause, onResume, onSkip, onStop, onRetry, onReplay, onNew }: {
  queue: DownloadQueue | null;
  input: string;
  notice: string;
  runtimeReady: boolean;
  singleDownloadActive: boolean;
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
  onReplay: () => void;
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
        <label className="queue-browser-picker">Авторизація через браузер<select disabled={!settingsEditable} value={settings.cookiesBrowser || ""} onChange={(event) => onSettings({ cookiesBrowser: event.target.value || null })}><option value="">Не використовувати cookies</option><option value="edge">Microsoft Edge</option><option value="chrome">Google Chrome</option><option value="firefox">Firefox</option><option value="brave">Brave</option><option value="vivaldi">Vivaldi</option></select><small>Потрібно лише для сайтів, які просять увійти або підтвердити вік.</small></label>
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
          : item.finalSize > 0 ? <span className={`queue-item-result partial ${item.status}`}>{formatByteSize(item.finalSize)}<small>{queueStatusLabel(item)}</small></span>
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
      {(!queue || queue.status === "draft") && <button className="primary-button" disabled={(!queue?.items.length && !input.trim()) || !runtimeReady || singleDownloadActive} onClick={onStart}><Icon name="download"/>Почати пакетне завантаження</button>}
      {queue?.status === "running" && <><button className="secondary-button" onClick={onPause}>Пауза після поточного</button><button className="secondary-button" disabled={!activeItem} onClick={onSkip}>Пропустити поточне</button><button className="cancel-confirm-button" onClick={onStop}>Зупинити все</button></>}
      {queue?.status === "paused" && <><button className="primary-button compact" disabled={singleDownloadActive || Boolean(activeItem)} onClick={onResume}><Icon name="download"/>{activeItem ? "Завершуємо поточний файл…" : "Продовжити"}</button>{activeItem && <button className="secondary-button" onClick={onSkip}>Пропустити поточне</button>}<button className="queue-new-button" disabled={Boolean(activeItem)} onClick={onNew}>Нова черга</button></>}
      {queue?.status === "completed" && <>{failedCount > 0 && <button className="secondary-button" onClick={onRetry}>Спробувати проблемні ({failedCount})</button>}<button className="primary-button compact" onClick={onReplay}><Icon name="refresh"/>Повторити чергу</button><button className="queue-new-button" onClick={onNew}>Нова черга</button></>}
    </div>
  </section>;
}

export function QueueProgressModal({ queue, alert, singleDownloadActive, onClose, onPause, onResume, onSkip, onStop, onRetry, onReplay, onNew }: {
  queue: DownloadQueue;
  alert: { title: string; message: string } | null;
  singleDownloadActive: boolean;
  onClose: () => void;
  onPause: () => void;
  onResume: () => void;
  onSkip: () => void;
  onStop: () => void;
  onRetry: () => void;
  onReplay: () => void;
  onNew: () => void;
}) {
  const progress = queueProgress(queue.items);
  const activeItem = queue.items.find((item) => item.id === queue.activeItemId) || null;
  const failedCount = queue.items.filter((item) => item.status === "failed" || item.status === "interrupted").length;
  const skippedCount = queue.items.filter((item) => item.status === "skipped").length;
  const statusTitle = queue.status === "running"
    ? activeItem ? "Завантажуємо чергу" : "Готуємо наступне посилання"
    : queue.status === "paused" ? "Чергу призупинено" : "Чергу оброблено";

  return <div className="download-progress-backdrop queue-progress-backdrop" role="presentation">
    <DialogSurface className={`queue-progress-modal ${queue.status}`} labelledBy="queue-progress-title" onEscape={onClose}>
      <header className="queue-progress-header">
        <div className="queue-progress-symbol"><Icon name={queue.status === "completed" ? "check" : "list"} size={24}/></div>
        <div>
          <p className="eyebrow">ПАКЕТНЕ ЗАВАНТАЖЕННЯ</p>
          <h2 id="queue-progress-title">{statusTitle}</h2>
          <p>{progress.done} із {progress.total} оброблено{failedCount ? ` · проблемних ${failedCount}` : ""}{skippedCount ? ` · пропущено ${skippedCount}` : ""}</p>
        </div>
        <strong className="queue-progress-value">{progress.percent}%</strong>
        <button type="button" className="queue-progress-close" aria-label="Згорнути стан черги" title="Згорнути" onClick={onClose}><Icon name="x" size={18}/></button>
      </header>
      <div className="queue-modal-progress" role="progressbar" aria-label="Загальний прогрес черги" aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress.percent}><span style={{ width: `${progress.percent}%` }}/></div>
      {alert && <div className="queue-progress-alert"><Icon name={queue.status === "completed" ? "check" : "shield"} size={18}/><span><strong>{alert.title}</strong><small>{alert.message}</small></span></div>}
      <div className="queue-progress-list">
        {queue.items.map((item, index) => {
          const active = item.id === activeItem?.id;
          return <article className={`queue-progress-row ${item.status} ${active ? "active" : ""}`} key={item.id}>
            <span className="queue-progress-index">{String(index + 1).padStart(2, "0")}</span>
            <div className="queue-progress-copy">
              <strong title={item.title || item.url}>{item.title || item.url}</strong>
              <small>{queueStatusLabel(item)}{item.message && item.message !== queueStatusLabel(item) ? ` · ${item.message}` : ""}</small>
              {!activeItem && ["failed", "interrupted"].includes(item.status) && item.jobId && <DownloadTools url={item.url} jobId={item.jobId} canDiscover={canOfferBrowserDiscovery(item.errorCode)}/>}
              {active && <div className={`queue-item-progress ${item.status === "postprocessing" ? "indeterminate" : ""}`}><span style={item.status === "postprocessing" ? undefined : { width: `${item.percent}%` }}/></div>}
            </div>
            <span className={`queue-progress-status ${item.status}`}>{item.finalSize > 0 ? <>{formatByteSize(item.finalSize)}{item.status !== "completed" && <small>{queueStatusLabel(item)}</small>}</> : ["downloading", "converting"].includes(item.status) ? `${Math.round(item.percent)}%` : queueStatusLabel(item)}</span>
          </article>;
        })}
      </div>
      <footer className="queue-modal-actions">
        {queue.status === "running" && <><button className="secondary-button" onClick={onPause}>Пауза після поточного</button><button className="secondary-button" disabled={!activeItem} onClick={onSkip}>Пропустити поточне</button><button className="cancel-confirm-button" onClick={onStop}>Зупинити все</button></>}
        {queue.status === "paused" && <><button className="primary-button compact" disabled={singleDownloadActive || Boolean(activeItem)} onClick={onResume}><Icon name="download"/>{activeItem ? "Завершуємо поточний файл…" : "Продовжити"}</button>{activeItem && <button className="secondary-button" onClick={onSkip}>Пропустити поточне</button>}<button className="queue-new-button" disabled={Boolean(activeItem)} onClick={onNew}>Нова черга</button></>}
        {queue.status === "completed" && <>{failedCount > 0 && <button className="secondary-button" onClick={onRetry}>Проблемні ще раз ({failedCount})</button>}<button className="primary-button compact" onClick={onReplay}><Icon name="refresh"/>Повторити чергу</button><button className="queue-new-button" onClick={onNew}>Нова черга</button></>}
      </footer>
    </DialogSurface>
  </div>;
}
