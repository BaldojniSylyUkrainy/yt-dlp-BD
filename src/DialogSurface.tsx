import { useEffect, useRef, type ReactNode } from "react";

export function DialogSurface({ className, labelledBy, describedBy, onEscape, children }: {
  className: string; labelledBy: string; describedBy?: string; onEscape?: () => void; children: ReactNode;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const escapeRef = useRef(onEscape);
  escapeRef.current = onEscape;
  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const focusable = () => Array.from(dialog.querySelectorAll<HTMLElement>("button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [href], [tabindex]:not([tabindex='-1'])"));
    const frame = window.requestAnimationFrame(() => (focusable()[0] || dialog).focus());
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && escapeRef.current) { event.preventDefault(); escapeRef.current(); return; }
      if (event.key !== "Tab") return;
      const controls = focusable();
      if (!controls.length) { event.preventDefault(); dialog.focus(); return; }
      const first = controls[0], last = controls[controls.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
    };
    dialog.addEventListener("keydown", keydown);
    return () => { window.cancelAnimationFrame(frame); dialog.removeEventListener("keydown", keydown); previous?.focus(); };
  }, []);
  return <div ref={dialogRef} className={className} role="dialog" aria-modal="true" aria-labelledby={labelledBy} aria-describedby={describedBy} tabIndex={-1}>{children}</div>;
}
