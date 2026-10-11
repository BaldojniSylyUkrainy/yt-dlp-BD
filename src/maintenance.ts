export const RETRY_DELAY_MS = 60 * 60 * 1000;

export function maintenanceDue(storage: Pick<Storage, "getItem">, key: string, interval: number, now: number): boolean {
  const success = Number(storage.getItem(key) || 0);
  const attempt = Number(storage.getItem(`${key}.attempt`) || 0);
  const retryAt = Number(storage.getItem(`${key}.retryAt`) || 0);
  return now >= retryAt && now - success >= interval && now - attempt >= RETRY_DELAY_MS;
}

export function maintenanceRetryAt(error: unknown, now: number): number {
  const server = /retry_at=(\d+)/.exec(String(error));
  return Math.max(now + RETRY_DELAY_MS, Math.min(Number(server?.[1] || 0) * 1000, now + 24 * RETRY_DELAY_MS));
}

export function friendlyMaintenanceError(error: unknown): string {
  return String(error).replace(/\s*\[retry_at=\d+\]/g, "");
}
