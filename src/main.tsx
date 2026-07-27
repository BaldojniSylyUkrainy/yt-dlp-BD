import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

const rootElement = document.getElementById("root") as HTMLElement;
const bootShell = document.getElementById("boot-shell");
const bootObserver = new MutationObserver(() => {
  if (!rootElement.childElementCount) return;
  bootShell?.remove();
  bootObserver.disconnect();
});
bootObserver.observe(rootElement, { childList: true });

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
