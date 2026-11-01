// @vitest-environment happy-dom
import {afterEach, expect, it} from "vitest";
import {cleanup, render} from "@testing-library/react";
import {RuntimePreparationOverlay, type RuntimeStatus} from "./RuntimeViews";
afterEach(cleanup);

const installed = {installed: true, version: "1", path: "/runtime/x", managed: true};
const runtime: RuntimeStatus = {ytDlp: installed, ffmpeg: installed, deno: installed, runtimeDir: "/runtime", platform: "windows"};

function states(view: ReturnType<typeof render>) {
  return [...view.container.querySelectorAll(".runtime-preparation-list > div")].map((row) => `${row.querySelector("strong")?.textContent}: ${row.querySelector("small")?.textContent}`);
}

it("does not call a component ready while it is still queued in this run", () => {
  const view = render(<RuntimePreparationOverlay stage="ffmpeg" runtime={runtime} progress={{component: "ffmpeg", downloaded: 51, total: 100}} pending={["deno"]}/>);
  expect(states(view)).toEqual(["yt-dlp: Готово", "ffmpeg: 51%", "Deno: У черзі"]);
  expect(view.container.querySelectorAll(".ready")).toHaveLength(1);
});

it("shows finished components as ready once nothing is queued", () => {
  const view = render(<RuntimePreparationOverlay stage="deno" runtime={runtime} progress={null} pending={[]}/>);
  expect(states(view)).toEqual(["yt-dlp: Готово", "ffmpeg: Готово", "Deno: Встановлюємо…"]);
});
