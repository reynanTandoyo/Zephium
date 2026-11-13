import "$styles/global.css";
import { afterEach, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page, userEvent } from "vitest/browser";
import type {
  WorkModelProvider,
  WorkModelReasoning,
  WorkModelRole,
  WorkModelsV1,
} from "$shared/ipc/bindings";
import { preferences } from "$domain/preferences";
import { PROFILE, models } from "$shared/testing/work-models";
import ModelPickerStage from "./ModelPickerStage.svelte";

const native = vi.hoisted(() => ({
  view: null as unknown as WorkModelsV1,
  ready: true,
  settings: {} as Record<string, string>,
}));

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    workModels: vi.fn(async () => native.view),
    workModelsReady: vi.fn(async (profile: string) => ({
      version: 1,
      profile,
      ready: native.ready,
      missing_roles: (native.ready ? [] : ["lead"]) as WorkModelRole[],
      fault: null,
    })),
    workSetModelEffort: vi.fn(async (_profile: string, effort: WorkModelReasoning | null) => {
      native.view = { ...native.view, effort };
      return native.view;
    }),
    workMoreModels: vi.fn(async (_profile: string, provider: WorkModelProvider) => ({
      provider,
      entries: [],
      fault: null,
    })),
    settingGet: async (key: string) => native.settings[key] ?? null,
    settingSet: async (key: string, value: string) => {
      native.settings[key] = value;
      return { accepted: true, operation_id: null };
    },
  });
});

const shots = "../../../../../target/look/model-picker";
const settle = (ms = 450) => new Promise((done) => setTimeout(done, ms));
const keyed = (lead: string) =>
  models({
    apps: { claude_code: "ready", antigravity: "ready", open_code: "sign_in_needed" },
    lead,
  });

/** Glass is the menus' own material; solid is the opaque policy a Windows sidebar asks for. */
async function shoot(name: string, materials: readonly ("glass" | "solid")[] = ["glass"]) {
  for (const theme of ["dark", "light"]) {
    for (const material of materials) {
      document.documentElement.dataset.theme = theme;
      const menu = document.querySelector(".model-menu");
      if (material === "solid") menu?.setAttribute("data-menu-material", "opaque");
      else menu?.removeAttribute("data-menu-material");
      await settle();
      const suffix = materials.length > 1 ? `-${material}` : "";
      await page.screenshot({ path: `${shots}/${name}-${theme}${suffix}.png` });
    }
  }
  document.documentElement.dataset.theme = "dark";
  document.querySelector(".model-menu")?.removeAttribute("data-menu-material");
}

afterEach(() => {
  native.ready = true;
  native.settings = {};
  preferences.dispose();
});

const rail = (name: RegExp) => page.getByRole("tab", { name });

test(
  "the chip, the card, a row's star, favourites, search, and the apps that wait on a step",
  { timeout: 60_000 },
  async () => {
    await page.viewport(1100, 700);
    native.view = keyed("claude-code/opus");
    native.settings["work.favourite-models"] = "claude-code/sonnet\nantigravity/gemini-3-pro";
    await preferences.init();
    const screen = await render(ModelPickerStage, { profile: PROFILE });
    await shoot("rest");
    await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
    await shoot("card", ["glass", "solid"]);
    await userEvent.hover(page.getByRole("option", { name: /Claude Code · Sonnet/u }));
    await shoot("hover-row");
    await userEvent.unhover(page.getByRole("listbox"));
    await page.getByRole("button", { name: /^Effort: Medium/u }).click();
    await shoot("effort-face");
    await userEvent.keyboard("{Escape}");
    await page.getByRole("button", { name: "Favourites", exact: true }).click();
    await shoot("favourites");
    await rail(/^Antigravity/u).click();
    await shoot("antigravity");
    await rail(/^OpenCode/u).click();
    await shoot("opencode-sign-in");
    await rail(/^Custom/u).click();
    await shoot("server");
    await rail(/^Claude Code/u).click();
    await userEvent.keyboard("son");
    await shoot("search");
    await userEvent.keyboard("zzz");
    await shoot("search-empty");
  },
);

test("the effort at every level, from the slider's keys", async () => {
  await page.viewport(1100, 700);
  native.view = { ...keyed("claude-code/sonnet"), effort: "low" };
  const screen = await render(ModelPickerStage, { profile: PROFILE });
  await screen.getByRole("button", { name: /Model: Claude Code · Sonnet/u }).click();
  await page.getByRole("button", { name: /^Effort: Low/u }).click();
  const levels = ["low", "medium", "high", "xhigh", "max"];
  for (const [index, level] of levels.entries()) {
    if (index) await userEvent.keyboard("{ArrowRight}");
    await shoot(`effort-${level}`, level === "max" ? ["glass", "solid"] : ["glass"]);
  }
  await settle(1200);
  await shoot("rest-max");
});

test("a model with three efforts, and one with none", async () => {
  await page.viewport(1100, 700);
  native.view = keyed("antigravity/gemini-3-pro");
  const screen = await render(ModelPickerStage, { profile: PROFILE });
  await screen.getByRole("button", { name: /Model: Antigravity · Gemini 3 Pro/u }).click();
  await shoot("two-levels");
  await page.getByRole("button", { name: /^Effort:/u }).click();
  await userEvent.keyboard("{ArrowRight}");
  await shoot("two-levels-high");
  await userEvent.keyboard("{Escape}");
  await screen.unmount();

  native.view = models({
    apps: { open_code: "ready" },
    lead: "opencode/big-pickle",
  });
  native.view.entries.push({
    ...native.view.entries[0]!,
    id: "opencode/big-pickle",
    model: { provider: "open_code", wire: "cli", model: "big-pickle" },
    display_name: "Big Pickle",
  });
  const plain = await render(ModelPickerStage, { profile: PROFILE });
  await plain.getByRole("button", { name: /Model: OpenCode · Big Pickle/u }).click();
  await shoot("no-effort");
});

test("keyless", async () => {
  await page.viewport(1100, 700);
  native.view = models({});
  native.ready = false;
  const keyless = await render(ModelPickerStage, { profile: PROFILE });
  await shoot("keyless");
  await keyless.unmount();
});
