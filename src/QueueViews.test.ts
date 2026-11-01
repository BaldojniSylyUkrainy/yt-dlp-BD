import { expect, it } from "vitest";
import { queueItemDetail } from "./QueueViews";
import { createQueueItem } from "./queue";

it("shows speed and time left for the running queue item", () => {
  const item = { ...createQueueItem("https://example.com/a", "a"), status: "downloading" as const, percent: 15, speed: "4.66MiB/s", eta: "03:10", message: "Завантаження…" };
  expect(queueItemDetail(item)).toBe("Завантаження 15% · 4.66MiB/s · залишилось 03:10");
  expect(queueItemDetail({ ...item, message: "Файл 2 із 5", eta: "—" })).toBe("Завантаження 15% · Файл 2 із 5 · 4.66MiB/s");
});

it("does not show a stale speed once the item has finished", () => {
  const item = { ...createQueueItem("https://example.com/a", "a"), status: "completed" as const, speed: "4.66MiB/s", eta: "00:01", message: "Готово" };
  expect(queueItemDetail(item)).toBe("Готово");
});
