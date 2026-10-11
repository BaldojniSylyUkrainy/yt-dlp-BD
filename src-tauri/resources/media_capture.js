(() => {
  if (window.__baldojCapture) return;
  window.__baldojCapture = true;
  const found = new Set();
  let timer;
  const flush = () => {
    if (!found.size) return;
    // Local custom scheme intercepted by native on_navigation (always denied).
    // It has no network host/HTTP request and no remote Tauri IPC permissions.
    location.href = 'baldoj-media://capture/?payload=' + encodeURIComponent(JSON.stringify({urls: [...found].slice(0, 30)}));
  };
  const add = (value, contentType = '') => {
    try {
      const url = new URL(value, location.href);
      if (!/^https?:$/.test(url.protocol) || url.username || url.password) return;
      if (!/\.(mp4|m3u8|mpd)(\?|$)/i.test(url.href) && !/video\/|mpegurl|dash\+xml/i.test(contentType)) return;
      if (found.has(url.href) || found.size >= 30 || url.href.length > 8192) return;
      found.add(url.href); clearTimeout(timer); timer = setTimeout(flush, 400);
    } catch { /* blob URLs aren't downloadable network addresses. */ }
  };
  new PerformanceObserver(list => list.getEntries().forEach(e => add(e.name))).observe({entryTypes: ['resource']});
  const originalFetch = window.fetch;
  window.fetch = async function(...args) {
    const response = await originalFetch.apply(this, args);
    add(response.url, response.headers.get('content-type') || '');
    return response;
  };
  const send = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function(...args) {
    this.addEventListener('load', () => add(this.responseURL, this.getResponseHeader('content-type') || ''));
    return send.apply(this, args);
  };
  setInterval(() => document.querySelectorAll('video, video source').forEach(v => add(v.currentSrc || v.src)), 1000);
})();
