// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {cleanup, fireEvent, render} from "@testing-library/react";
import {HistoryThumbnail} from "./HistoryThumbnail";
vi.mock("@tauri-apps/api/core", () => ({convertFileSrc: (path: string) => `asset://cache/${path}`}));
afterEach(cleanup);

it("never falls back to a remote URL when the local cache fails", () => {
  const entry = {cachedThumbnailPath: "one.jpg", thumbnail: "https://127.0.0.1/private"};
  const view = render(<HistoryThumbnail entry={entry}/>);
  const image = view.container.querySelector("img")!;
  expect(image.getAttribute("src")).toBe("asset://cache/one.jpg");
  fireEvent.error(image);
  expect(view.container.querySelector("img")).toBeNull();
  view.rerender(<HistoryThumbnail entry={{...entry, cachedThumbnailPath: "two.jpg"}}/>);
  expect(view.container.querySelector("img")?.getAttribute("src")).toBe("asset://cache/two.jpg");
});

it("shows a local placeholder for uncached history", () => {
  const view = render(<HistoryThumbnail entry={{cachedThumbnailPath: null}}/>);
  expect(view.container.querySelector("img")).toBeNull();
  expect(view.container.querySelector("svg")).toBeTruthy();
});
