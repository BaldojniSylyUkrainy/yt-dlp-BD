import { useEffect, useState } from "react";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { formatByteSize } from "./format";
import { groupHistoryEntries, type HistoryEntry } from "./history";
import { HistoryThumbnail } from "./HistoryThumbnail";
import { Icon } from "./Icon";
import { HistoryDetails } from "./MediaTools";

export function HistoryView({ entries, retryingId, retryDisabled, onRetry, onClear }: {
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
              {entry.path ? <div className="history-location" title={entry.path}><span className={`history-state-dot ${entry.available ? "ready" : "missing"}`}/><span>{entry.available ? "Файл на місці" : "Файл недоступний"}</span><Icon name="folder" size={14}/><code>{entry.path}</code></div> : <p>{entry.outcome === "cancelled" ? "Скасовано" : entry.outcome === "auth_required" ? "Потрібен вхід" : "Не вдалося завантажити"} · {entry.message}</p>}
            </div>
            <div className="history-actions">
              <button type="button" className="history-retry-button" disabled={retryDisabled || retryingId !== null} onClick={() => onRetry(entry)}>{retryingId === entry.id ? "Готуємо…" : "Завантажити ще раз"}</button>
              {entry.path && <button type="button" disabled={!entry.available} onClick={() => revealItemInDir(entry.path)}>Показати</button>}
              <button type="button" onClick={() => openUrl(entry.sourceUrl)}>Джерело</button>
              <HistoryDetails title={entry.title} diagnostics={entry.diagnostics}/>
            </div>
          </article>)}
        </div>
      </details>)}
    </div>
  </section>;
}
