import { useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";

// Remote metadata is never a browser image source. Only the backend's guarded
// raster cache is exposed through the narrowly scoped Tauri asset protocol.
export function HistoryThumbnail({ entry }: { entry: { cachedThumbnailPath: string | null } }) {
  const cached = entry.cachedThumbnailPath ? convertFileSrc(entry.cachedThumbnailPath) : null;
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [cached]);
  return <div className="history-thumb">
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true"><path d="M12 3v12m-5-5 5 5 5-5M5 21h14"/></svg>
    {cached && !failed && <img src={cached} alt="" onError={() => setFailed(true)} />}
  </div>;
}
