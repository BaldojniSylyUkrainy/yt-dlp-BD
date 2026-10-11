import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { DialogSurface } from "./DialogSurface";

export function canOfferBrowserDiscovery(errorCode?: string | null): boolean {
  return ["unsupported", "unknown", "selection_required"].includes(errorCode || "unknown");
}

export function DownloadTools({ url, jobId, canDiscover = false }: { url: string; jobId: string; canDiscover?: boolean }) {
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    const stop = listen<{jobId: string}>("media-session-expired", ({payload}) => {
      if (payload.jobId === jobId) setMessage("Час пошуку минув. Натисніть «Знайти відео в браузері», щоб відкрити нову захищену сесію.");
    });
    return () => { void stop.then(fn => fn()); };
  }, [jobId]);
  if (!canDiscover) return null;
  return <div className="download-tools">
    <div className="discovery-help">
      <strong>Не вдалося отримати відео автоматично. Спробуємо через плеєр сайту?</strong>
      <ol>
        <li>Відкрийте сторінку кнопкою нижче.</li>
        <li>На сайті натисніть ▶ на потрібному відео й дочекайтеся відтворення.</li>
        <li>Коли з’явиться «Джерело знайдено», поверніться в застосунок і виберіть відео для завантаження.</li>
      </ol>
      <small>Посилання копіювати не потрібно. Це запасний спосіб: він працює не на кожному сайті й не обходить захист відео.</small>
    </div>
    {canDiscover && <button type="button" disabled={busy} onClick={async () => {
      setBusy(true);
      try { await invoke("open_media_browser", { pageUrl: url, jobId }); setMessage("У вікні пошуку натисніть ▶ на потрібному відео. Після виявлення джерела натисніть «Повернутися до завантаження» у верхній підказці."); }
      catch (error) { setMessage(String(error)); }
      finally { setBusy(false); }
    }}>Знайти відео в браузері</button>}
    {message && <small role="status">{message}</small>}
  </div>;
}

export function HistoryDetails({ title, diagnostics }: { title: string; diagnostics?: string }) {
  const [open, setOpen] = useState(false);
  const [message, setMessage] = useState("");
  return <>
    <button type="button" onClick={() => { setMessage(""); setOpen(true); }}>Подробиці</button>
    {open && <div className="utility-backdrop"><DialogSurface className="utility-dialog" labelledBy="history-diagnostics-title" onEscape={() => setOpen(false)}>
      <h2 id="history-diagnostics-title">Технічні подробиці</h2>
      <p>{title}</p>
      <p>Очищений журнал зберігається локально для 20 останніх записів. Він не надсилається автоматично.</p>
      {diagnostics ? <><textarea aria-label="Діагностика" readOnly value={diagnostics}/><button type="button" onClick={async () => {
        try { await navigator.clipboard.writeText(diagnostics); setMessage("Діагностику скопійовано"); }
        catch { setMessage("Виділіть і скопіюйте текст вручну"); }
      }}>Скопіювати діагностику</button></> : <p>Журнал цього завантаження не збережено або він уже застарів.</p>}
      {message && <small role="status">{message}</small>}
      <button type="button" onClick={() => setOpen(false)}>Закрити подробиці</button>
    </DialogSurface></div>}
  </>;
}

export type Candidates = { sessionId: string; jobId: string; pageUrl: string; urls: string[] };
export function MediaCandidatePicker({ busy, onSelected, isCurrent = () => true }: { busy: boolean; onSelected: (candidate: Candidates) => void; isCurrent?: (candidate: Candidates) => boolean }) {
  const [candidates, setCandidates] = useState<Candidates | null>(null);
  const [error, setError] = useState("");
  const [selecting, setSelecting] = useState(false);
  useEffect(() => {
    const unlisten = listen<Candidates>("media-candidates", ({ payload }) => { setCandidates(payload); setError(""); });
    const expiry = listen<{sessionId: string}>("media-session-expired", ({payload}) => {
      setCandidates(current => current?.sessionId === payload.sessionId ? null : current);
    });
    return () => { void unlisten.then((stop) => stop()); void expiry.then(stop => stop()); };
  }, []);
  if (!candidates || busy || !isCurrent(candidates)) return null;
  const dismiss = () => { void invoke("dismiss_media_candidates", {sessionId: candidates.sessionId}).catch(() => undefined); setCandidates(null); };
  return <div className="utility-backdrop"><DialogSurface className="utility-dialog" labelledBy="media-candidate-title" onEscape={selecting ? undefined : dismiss}>
    <h2 id="media-candidate-title">Знайдені джерела відео</h2>
    <p>Оберіть потрібний плеєр. Серед джерел може бути реклама; найкраща доступна якість усередині обраного потоку визначається автоматично. DRM не обходиться.</p>
    {candidates.urls.map((url, index) => {
      const parsed = new URL(url);
      return <button type="button" disabled={selecting} key={url} onClick={async () => {
        setSelecting(true);
        try { await invoke("select_media_candidate", { sessionId: candidates.sessionId, index }); onSelected(candidates); setCandidates(null); }
        catch (error) { setError(String(error)); }
        finally { setSelecting(false); }
      }}>{index + 1}. {parsed.hostname} · {parsed.pathname.split("/").pop()?.slice(0, 90) || "Медіапотік"}</button>;
    })}
    {error && <p role="alert">{error}</p>}
    <button type="button" disabled={selecting} onClick={dismiss}>Пізніше</button>
  </DialogSurface></div>;
}

export function RecoveryNotice({ ready }: { ready: boolean }) {
  const [items, setItems] = useState<{ id: string; directory: string }[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => { if (ready) void invoke<typeof items>("interrupted_work").then(setItems).catch(() => undefined); }, [ready]);
  if (!ready || !items.length) return null;
  return <section className="recovery-notice" aria-label="Незавершені завантаження">
    <strong>Залишилися тимчасові файли незавершених завантажень</strong>
    <p>Видаляються лише зазначені службові папки. Готові відео не зачіпаємо.</p>
    {items.map((item) => <div key={item.id}><code>{item.directory}</code><button type="button" disabled={busy} onClick={async () => {
      setBusy(true);
      try { await invoke("remove_interrupted_work", { id: item.id }); setItems((items) => items.filter((i) => i.id !== item.id)); }
      catch (error) { setError(String(error)); }
      finally { setBusy(false); }
    }}>Видалити залишки</button></div>)}
    {error && <p role="alert">{error}</p>}
    <button type="button" onClick={() => setItems([])}>Не зараз</button>
  </section>;
}
