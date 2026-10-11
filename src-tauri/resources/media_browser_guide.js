(() => {
  if (window.__baldojGuide) return;
  window.__baldojGuide = true;
  const topFrame = window === window.top;
  let sourceFound = false;
  let expired = false;
  let status;
  let back;
  const update = () => {
    if (!status) return;
    status.textContent = expired ? '15 хвилин пошуку минули. З’єднання закрито для безпеки. Поверніться до застосунку й відкрийте пошук повторно.' : sourceFound
      ? 'Джерело знайдено. Поверніться до застосунку й оберіть потрібне відео.'
      : 'Натисніть ▶ на потрібному відео на сайті нижче. Дочекайтеся початку відтворення — посилання шукатимемо самі.';
    back.textContent = expired ? 'Повернутися до застосунку →' : sourceFound ? 'Повернутися до завантаження →' : 'Повернутися без результату';
    back.classList.toggle('ready', sourceFound || expired);
    status.toggleAttribute('data-found', sourceFound && !expired);
  };
  window.addEventListener('baldoj-source-found', () => { sourceFound = true; update(); });
  window.addEventListener('baldoj-session-expired', () => { expired = true; update(); });
  const mount = () => {
    if (!document.documentElement || !document.body) return;
    if (!topFrame) return;
    const space = document.createElement('style');
    space.textContent = 'html { scroll-padding-top: var(--baldoj-guide-height, 150px) !important; } body { padding-top: var(--baldoj-guide-height, 150px) !important; }';
    (document.head || document.documentElement).append(space);
    const host = document.createElement('div');
    host.id = 'baldoj-browser-guide';
    host.style.cssText = 'all:initial!important;position:fixed!important;inset:0 0 auto 0!important;display:block!important;z-index:2147483647!important;';
    const root = host.attachShadow({mode: 'closed'});
    const style = document.createElement('style');
    style.textContent = `
      :host { color-scheme: dark; }
      .bar { box-sizing:border-box; background:#171417; color:#f6f1f4; border-bottom:3px solid #ba789b; padding:14px 20px; font:15px/1.45 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; }
      strong { display:block; color:#e0aecb; font-size:16px; } p { margin:6px 0 12px; }
      .actions { display:flex; flex-wrap:wrap; gap:8px; }
      button { font:inherit; padding:8px 12px; border:1px solid #80747d; background:#302a2e; color:#fff; cursor:pointer; border-radius:4px; }
      button:focus-visible { outline:3px solid #fff; outline-offset:2px; }
      .ready { background:#ba789b; color:#171017; font-weight:700; }
      [data-found] { color:#9be3b2; }
    `;
    const bar = document.createElement('section');
    bar.className = 'bar';
    bar.setAttribute('aria-label', 'Підказка Baldojnyi Downloader');
    const title = document.createElement('strong');
    title.textContent = 'Запасний пошук відео • Baldojnyi Downloader';
    status = document.createElement('p');
    status.setAttribute('role', 'status');
    const actions = document.createElement('div');
    actions.className = 'actions';
    back = document.createElement('button');
    back.type = 'button';
    back.addEventListener('click', () => { location.href = 'baldoj-media://return/'; });
    actions.append(back);
    bar.append(title, status, actions);
    root.append(style, bar);
    document.documentElement.append(host);
    update();
    const reserve = () => document.documentElement.style.setProperty('--baldoj-guide-height', `${Math.ceil(host.getBoundingClientRect().height) + 12}px`);
    new ResizeObserver(reserve).observe(host);
    reserve();
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount, {once: true});
  else mount();
})();
