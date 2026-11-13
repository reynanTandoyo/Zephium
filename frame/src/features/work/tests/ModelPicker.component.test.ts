import "$styles/global.css";
import { expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page, userEvent } from "vitest/browser";
import type { WorkModelReasoning, WorkModelsReadyV1 } from "$shared/ipc/bindings";
import { preferences } from "$domain/preferences";
import ModelPicker from "../components/bar/ModelPicker.svelte";
import ModelBarStage from "./ModelBarStage.svelte";
import { PROFILE, entry, models } from "$shared/testing/work-models";

const native = vi.hoisted(() => ({
  read: vi.fn(),
  choose: vi.fn(),
  effort: vi.fn(),
  more: vi.fn(),
  command: vi.fn(async () => ({ operation_id: "op", accepted: true })),
  settings: {} as Record<string, string>,
  ready: vi.fn(async (profile: string): Promise<WorkModelsReadyV1> => ({
    version: 1,
    profile,
    ready: true,
    missing_roles: [],
    fault: null,
  })),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    workModels: native.read,
    workChooseModel: native.choose,
    workSetModelEffort: native.effort,
    workMoreModels: native.more,
    workModelsReady: native.ready,
    runCommand: native.command,
    settingGet: async (key: string) => native.settings[key] ?? null,
    settingSet: async (key: string, value: string) => {
      native.settings[key] = value;
      return { accepted: true, operation_id: null };
    },
  });
});

const view = (lead: string, chosen: string | null = null, effort?: WorkModelReasoning) =>
  models({
    apps: { claude_code: "ready", antigravity: "ready" },
    lead,
    chosen,
    effort,
  });

const ready = (value: boolean) => async (profile: string) => ({
  version: 1,
  profile,
  ready: value,
  missing_roles: value ? [] : (["lead"] as const).slice(),
  fault: null,
});

test("the trigger is the model's mark and words, and the menu changes the model", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  native.choose.mockResolvedValue(view("antigravity/gemini-3-pro", "antigravity/gemini-3-pro"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });

  const trigger = screen.getByRole("button", { name: "Model: Claude Code · Opus, Medium effort" });
  await expect.element(trigger).toHaveTextContent(/Claude Code · Opus\s*·\s*Medium/u);
  expect(trigger.element().querySelector("svg")).not.toBeNull();
  await trigger.click();

  await expect.element(page.getByRole("listbox")).toHaveFocus();
  const card = page.getByRole("region", { name: "Claude Code · Opus" });
  await expect.element(card).toHaveTextContent(/Uses your Claude Code account/u);
  await expect
    .element(page.getByRole("option", { name: /Claude Code · Opus/u }))
    .toHaveAttribute("aria-selected", "true");
  await expect
    .element(page.getByRole("tab", { name: "Claude Code" }))
    .toHaveAttribute("aria-selected", "true");
  // Only the running model's provider is listed until another is chosen.
  expect(page.getByRole("option", { name: /Gemini 3 Pro/u }).query()).toBeNull();

  await page.getByRole("tab", { name: "Antigravity" }).click();
  await page.getByRole("option", { name: /Gemini 3 Pro/u }).click();
  expect(native.choose).toHaveBeenCalledWith(PROFILE, "lead", "antigravity/gemini-3-pro");
  await expect
    .element(screen.getByRole("button", { name: /Model: Antigravity · Gemini 3 Pro/u }))
    .toBeVisible();
});

test("the rail holds the AI apps and the custom server, each with its own state", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();

  const names = page
    .getByRole("tab")
    .all()
    .map((tab) => tab.element().getAttribute("aria-label"));
  // The built-in key providers are not offered.
  expect(names).toEqual([
    "Claude Code",
    "Antigravity",
    "OpenCode, set up in Settings",
    "Custom server, connect a server",
  ]);
});

test("an app that is not ready leads to Settings", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  const onsettings = vi.fn();
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings });
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();

  await page.getByRole("tab", { name: "OpenCode, set up in Settings" }).click();
  await expect.element(page.getByText("OpenCode isn’t ready yet", { exact: false })).toBeVisible();
  expect(page.getByRole("option").query()).toBeNull();
  await page.getByRole("button", { name: "Open Settings" }).click();
  expect(onsettings).toHaveBeenCalledTimes(1);

  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
  await page.getByRole("tab", { name: "Custom server, connect a server" }).click();
  await expect
    .element(page.getByText("Connect a server of your own", { exact: false }))
    .toBeVisible();
  document.querySelector<HTMLElement>(".model-menu [role=tabpanel]")!.focus();
  await userEvent.keyboard("{Enter}");
  expect(onsettings).toHaveBeenCalledTimes(2);
});

test("the header opens Settings, and no foot row asks for a key", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  const onsettings = vi.fn();
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings });
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
  expect(page.getByRole("button", { name: /Add a key/u }).query()).toBeNull();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  expect(onsettings).toHaveBeenCalledTimes(1);
});

test("an app that still needs a sign-in is listed apart, not among the ready ones", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(
    models({
      apps: { claude_code: "ready", antigravity: "sign_in_needed" },
      lead: "claude-code/opus",
    }),
  );
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
  await page.getByRole("tab", { name: "Antigravity, set up in Settings" }).click();
  await expect
    .element(page.getByText("Antigravity isn’t ready yet", { exact: false }))
    .toBeVisible();
  expect(page.getByRole("option").query()).toBeNull();
});

test("deferred opening autofocus preserves a provider already reached from the rail", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  let moved = false;
  let scopesAtNavigation = 0;
  const scopes = new Set<Event>();
  const originalPreventDefault = Event.prototype.preventDefault;
  const prevent = vi.spyOn(Event.prototype, "preventDefault").mockImplementation(function (
    this: Event,
  ) {
    if (this.type === "focusScope.onOpenAutoFocus") scopes.add(this);
    originalPreventDefault.call(this);
  });
  const navigate = (event: FocusEvent) => {
    const target = event.target;
    if (moved || !(target instanceof HTMLElement) || target.getAttribute("role") !== "listbox")
      return;
    moved = true;
    const tab = target
      .closest(".model-menu")
      ?.querySelector<HTMLButtonElement>('[role="tab"][data-provider="claude_code"]');
    queueMicrotask(() => {
      scopesAtNavigation = scopes.size;
      tab?.focus();
      tab?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }),
      );
    });
  };
  document.addEventListener("focusin", navigate, true);
  try {
    await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
    const selected = page.getByRole("tab", { name: "Antigravity" });
    await expect.element(selected).toHaveAttribute("aria-selected", "true");
    // Observe the actual second scope callback rather than racing its deferred focus.
    await expect.poll(() => scopes.size).toBeGreaterThanOrEqual(2);
    expect(moved).toBe(true);
    expect(scopesAtNavigation).toBe(1);
    expect(document.activeElement).toBe(selected.element());
  } finally {
    document.removeEventListener("focusin", navigate, true);
    prevent.mockRestore();
    await screen.unmount();
  }
});

test("the rail moves with the arrow keys, and arrowing the list moves the row Enter takes", async () => {
  await page.viewport(1200, 800);
  native.choose.mockClear();
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/sonnet"));
  native.choose.mockResolvedValue(view("claude-code/opus", "claude-code/opus"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Claude Code · Sonnet/u }).click();
  const list = page.getByRole("listbox");
  await expect.element(list).toHaveFocus();

  const claude = page.getByRole("tab", { name: "Claude Code" });
  (claude.element() as HTMLElement).focus();
  await userEvent.keyboard("{ArrowDown}");
  await expect.element(page.getByRole("tab", { name: "Antigravity" })).toHaveFocus();
  await expect.element(page.getByRole("option", { name: /Gemini 3 Pro/u })).toBeVisible();
  await userEvent.keyboard("{ArrowUp}");
  await expect.element(claude).toHaveFocus();
  await expect.element(page.getByRole("option", { name: /Claude Code · Opus/u })).toBeVisible();

  (list.element() as HTMLElement).focus();
  await userEvent.keyboard("{ArrowDown}");
  const opus = page.getByRole("option", { name: /Claude Code · Opus/u });
  await expect.element(list).toHaveAttribute("aria-activedescendant", opus.element().id);
  expect(native.choose).not.toHaveBeenCalled();
  await userEvent.keyboard("{Enter}");
  expect(native.choose).toHaveBeenCalledWith(PROFILE, "lead", "claude-code/opus");
});

test("typing slides the search in over every provider, Enter takes the first match, Escape puts it away", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("antigravity/gemini-3-pro"));
  native.choose.mockResolvedValue(view("claude-code/sonnet", "claude-code/sonnet"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Antigravity · Gemini 3 Pro/u }).click();

  await userEvent.keyboard("sonnet");
  const field = page.getByRole("combobox", { name: "Search models" });
  await expect.element(field).toHaveValue("sonnet");
  await expect.poll(() => page.getByRole("option").all().length).toBe(1);
  await expect.element(page.getByRole("group", { name: "Claude Code" })).toBeVisible();

  await userEvent.keyboard("{Escape}");
  await expect.element(field).toHaveValue("");
  await expect
    .element(page.getByRole("button", { name: "Search models" }))
    .toHaveAttribute("aria-expanded", "false");
  await expect.element(page.getByRole("option", { name: /Gemini 3 Pro/u })).toBeVisible();

  await userEvent.keyboard("sonnet{Enter}");
  expect(native.choose).toHaveBeenCalledWith(PROFILE, "lead", "claude-code/sonnet");
});

test("the clear button empties the search and returns to the provider's list", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("antigravity/gemini-3-pro"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Antigravity · Gemini 3 Pro/u }).click();
  await userEvent.keyboard("zzz");
  await expect.element(page.getByText("No model matches")).toBeVisible();
  await page.getByRole("button", { name: "Clear search" }).click();
  await expect.element(page.getByRole("combobox", { name: "Search models" })).toHaveValue("");
  await expect.element(page.getByRole("option", { name: /Gemini 3 Pro/u })).toBeVisible();
});

test("the effort shows in a word, and the card turns into its control to set it", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/sonnet"));
  native.effort.mockImplementation(async (_profile: string, effort: WorkModelReasoning) =>
    view("claude-code/sonnet", null, effort),
  );
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Claude Code · Sonnet, Medium effort/u }).click();

  // At rest the card is the model, with its effort in a word; the control is not built.
  const pill = page.getByRole("button", { name: "Effort: Medium" });
  await expect.element(pill).toHaveAttribute("aria-expanded", "false");
  expect(page.getByRole("slider").query()).toBeNull();
  expect(document.querySelector(".model-menu .core")).toBeNull();

  await pill.click();
  const effort = page.getByRole("slider", { name: "Effort" });
  await expect.element(effort).toHaveFocus();
  await expect.element(effort).toHaveAttribute("aria-valuemax", "4");
  await expect.element(effort).toHaveAttribute("aria-valuetext", "Medium");
  // The model is never shown beside it.
  expect(document.querySelector(".model-menu .face.model")?.hasAttribute("inert")).toBe(true);
  const pace = () =>
    document
      .querySelector(".model-menu .core")!
      .getAnimations({ subtree: true })
      .map((run) => run.playbackRate)[0];
  await expect.poll(pace).toBeGreaterThan(0);
  const calm = pace()!;

  await userEvent.keyboard("{End}");
  await expect.element(effort).toHaveAttribute("aria-valuetext", "Max");
  expect(native.effort).toHaveBeenLastCalledWith(PROFILE, "max");
  await expect.poll(pace).toBeGreaterThan(calm);
  await expect
    .element(screen.getByRole("button", { name: /Model: Claude Code · Sonnet, Max effort/u }))
    .toBeInTheDocument();

  // Used, it turns back to the model and hands the keys to the button that opened it.
  await expect.element(page.getByRole("button", { name: "Effort: Max" })).toHaveFocus();
  expect(document.querySelector(".model-menu .face.effort")?.hasAttribute("inert")).toBe(true);
});

test("Escape turns the open effort back first, and only then closes the menu", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/sonnet"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  const trigger = screen.getByRole("button", { name: /Model: Claude Code · Sonnet/u });
  await trigger.click();
  await page.getByRole("button", { name: "Effort: Medium" }).click();
  await expect.element(page.getByRole("slider", { name: "Effort" })).toBeVisible();
  await userEvent.keyboard("{Escape}");
  await expect
    .element(page.getByRole("button", { name: "Effort: Medium" }))
    .toHaveAttribute("aria-expanded", "false");
  await expect.element(page.getByRole("region", { name: "Claude Code · Sonnet" })).toBeVisible();
  await userEvent.keyboard("{Escape}");
  await expect
    .element(page.getByRole("region", { name: "Claude Code · Sonnet" }))
    .not.toBeInTheDocument();
});

test("a star on a row keeps the model under Favourites, across providers", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  await preferences.init();
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
  expect(page.getByRole("tab", { name: "Favourites" }).query()).toBeNull();
  const favourites = page.getByRole("button", { name: "Favourites", exact: true });
  await expect.element(favourites).toHaveAttribute("aria-pressed", "false");

  await page.getByRole("button", { name: "Add Claude Code · Sonnet to favourites" }).click();
  await page.getByRole("tab", { name: "Antigravity" }).click();
  await page.getByRole("button", { name: "Add Antigravity · Gemini 3 Pro to favourites" }).click();
  await favourites.click();
  await expect.element(favourites).toHaveAttribute("aria-pressed", "true");
  await expect.poll(() => page.getByRole("option").all().length).toBe(2);
  await expect.element(page.getByRole("group", { name: "Claude Code" })).toBeVisible();
  await expect.element(page.getByRole("group", { name: "Antigravity" })).toBeVisible();

  await page
    .getByRole("button", {
      name: "Remove Antigravity · Gemini 3 Pro from favourites",
    })
    .click();
  await expect.poll(() => page.getByRole("option").all().length).toBe(1);
  await expect.poll(() => native.settings["work.favourite-models"]).toBe("claude-code/sonnet");

  // The same button goes back to the provider that was open.
  await favourites.click();
  await expect.element(favourites).toHaveAttribute("aria-pressed", "false");
  await expect
    .element(page.getByRole("tab", { name: "Antigravity" }))
    .toHaveAttribute("aria-selected", "true");
  preferences.dispose();
});

test("a row shows no tier word, and its check stands at the end where the star takes its place", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
  const list = document.querySelector(".model-menu [role=listbox]")!;
  expect(list.textContent).not.toMatch(/Flagship|Balanced|Lightweight/u);
  const row = list.querySelector('[aria-selected="true"]')!;
  const slot = row.querySelector(".slot")!;
  expect(slot.querySelector(".check svg")).not.toBeNull();
  const edge = (element: Element) => element.getBoundingClientRect().right;
  expect(edge(slot)).toBeGreaterThan(edge(row.querySelector(".option-name")!));
  expect(row.firstElementChild?.classList.contains("option-name")).toBe(true);
});

test("an empty search puts itself away when the menu is clicked elsewhere", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
  const search = page.getByRole("button", { name: "Search models" });
  await search.click();
  await expect.element(search).toHaveAttribute("aria-expanded", "true");
  await page.getByRole("region", { name: "Claude Code · Opus" }).click();
  await expect.element(search).toHaveAttribute("aria-expanded", "false");

  // With words in it, the search stays until it is cleared or a place is picked.
  await search.click();
  await userEvent.keyboard("gemini");
  await page.getByRole("region", { name: "Claude Code · Opus" }).click();
  await expect.element(page.getByRole("combobox", { name: "Search models" })).toHaveValue("gemini");
  await page.getByRole("tab", { name: "Antigravity" }).click();
  await expect.element(search).toHaveAttribute("aria-expanded", "false");
  await expect.element(page.getByRole("combobox", { name: "Search models" })).toHaveValue("");
});

test("the effort control is set by a click and a drag, and turns back after a rest, a click elsewhere or Escape", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/sonnet"));
  native.effort.mockImplementation(async (_profile: string, effort: WorkModelReasoning) => {
    await new Promise((done) => setTimeout(done, 30));
    return view("claude-code/sonnet", null, effort);
  });
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Claude Code · Sonnet/u }).click();
  const pill = () => page.getByRole("button", { name: /^Effort:/u });
  const slider = page.getByRole("slider", { name: "Effort" });
  const asked = native.effort.mock.calls.length;
  const open = async () => {
    await pill().click();
    await expect.element(slider).toBeVisible();
  };

  await open();
  await slider.click({ position: { x: 96, y: 18 } });
  await expect.element(slider).toHaveAttribute("aria-valuetext", "Max");
  await slider.click({ position: { x: 6, y: 18 } });
  await expect.element(slider).toHaveAttribute("aria-valuetext", "Low");
  expect(native.effort.mock.calls.slice(asked).map((call) => call[1])).toEqual(["max", "low"]);

  // Dragged from the thumb, it follows the pointer and lands on the stop it is let go at.
  const thumb = page.elementLocator(document.querySelector(".model-menu .thumb")!);
  await userEvent.dragAndDrop(thumb, slider, { targetPosition: { x: 52, y: 18 } });
  await expect.element(slider).toHaveAttribute("aria-valuetext", "High");
  expect(document.querySelector(".model-menu .thumb")).not.toBeNull();

  // Used, it turns back a moment after the last change.
  await expect
    .poll(() => pill().element().getAttribute("aria-expanded"), { timeout: 4000 })
    .toBe("false");

  // Left alone, it turns back after a rest.
  const opened = performance.now();
  await open();
  await new Promise((done) => setTimeout(done, 1500));
  await expect.element(pill()).toHaveAttribute("aria-expanded", "true");
  await expect
    .poll(() => pill().element().getAttribute("aria-expanded"), { timeout: 4000 })
    .toBe("false");
  expect(performance.now() - opened).toBeGreaterThan(2500);

  // A click elsewhere in the menu puts it away at once.
  await open();
  await page.getByRole("tab", { name: "Claude Code" }).click();
  await expect.element(pill()).toHaveAttribute("aria-expanded", "false");
});

test("a model that sets no effort shows no effort control", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  const plain = models({ apps: { open_code: "ready" }, lead: "opencode/big-pickle" });
  plain.entries.push({
    ...plain.entries[0]!,
    id: "opencode/big-pickle",
    model: { provider: "open_code", wire: "cli", model: "big-pickle" },
    display_name: "Big Pickle",
  });
  native.read.mockResolvedValue(plain);
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: "Model: OpenCode · Big Pickle" }).click();
  await expect
    .element(page.getByRole("option", { name: /OpenCode · Big Pickle/u }))
    .toHaveAttribute("aria-selected", "true");
  expect(page.getByRole("slider").query()).toBeNull();
  await expect.element(page.getByText("Its own pace")).toBeVisible();
});

test("without any key the control is one calm way to add a key, straight to Settings", async () => {
  await page.viewport(1200, 800);
  native.read.mockResolvedValue(models({}));
  native.ready.mockImplementation(ready(false));
  const onsettings = vi.fn();
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings });
  await screen.getByRole("button", { name: "Set up a model" }).click();
  expect(onsettings).toHaveBeenCalledTimes(1);
  expect(page.getByRole("listbox").query()).toBeNull();
});

test("no hosted provider is pinned, and a keyless bar asks only for a model key", async () => {
  await page.viewport(1200, 800);
  native.read.mockResolvedValue(view("claude-code/opus"));
  native.ready.mockImplementation(ready(false));
  const keyless = await render(ModelPicker, { profile: PROFILE });
  await expect.element(keyless.getByRole("button", { name: "Set up a model" })).toBeVisible();
  await keyless.unmount();

  native.ready.mockImplementation(ready(true));
  const screen = await render(ModelPicker, { profile: PROFILE });
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
  // The rail leads with the person's own providers.
  const rail = page.getByRole("tab").all();
  expect(rail[0]!.element().getAttribute("aria-label")).toBe("Claude Code");
  expect(page.getByText("Zephium AI", { exact: false }).query()).toBeNull();
  expect(page.getByRole("button", { name: "Sign in" }).query()).toBeNull();
});

test("the menu says so when no chosen model can search the web", async () => {
  await page.viewport(1200, 800);
  native.read.mockResolvedValue(
    models({ apps: { claude_code: "ready" }, lead: "claude-code/sonnet" }),
  );
  native.ready.mockImplementation(async (profile: string) => ({
    version: 1,
    profile,
    ready: true,
    missing_roles: [],
    search: "unavailable",
    fault: null,
  }));
  const screen = await render(ModelPicker, { profile: PROFILE });
  await screen.getByRole("button", { name: /Model: Claude Code · Sonnet/u }).click();
  await expect.element(page.getByText("Web search is off", { exact: false })).toBeVisible();
});

test("choosing a model leaves the composer open and the cursor in the words", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  native.choose.mockResolvedValue(view("antigravity/gemini-3-pro", "antigravity/gemini-3-pro"));
  const screen = await render(ModelBarStage, { profile: PROFILE });
  const bar = () => screen.container.querySelector<HTMLElement>(".work-bar")!;
  const words = screen.getByRole("textbox");
  await words.click();
  await words.fill("Plan the trip");
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
  await page.getByRole("tab", { name: "Antigravity" }).click();
  await page.getByRole("option", { name: /Gemini 3 Pro/u }).click();
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
  await expect.element(words).toHaveFocus();
  expect(bar().dataset.engaged).toBe("true");
  expect(bar().dataset.mode).toBe("focus");
  await expect.element(words).toHaveValue("Plan the trip");
});

test("closing the menu from the chip leaves an empty composer engaged and focused", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  const screen = await render(ModelBarStage, { profile: PROFILE });
  const bar = () => screen.container.querySelector<HTMLElement>(".work-bar")!;
  const words = screen.getByRole("textbox");
  await words.click();
  const chip = screen.getByRole("button", { name: /Model: Claude Code · Opus/u });
  await chip.click();
  await expect.element(page.getByRole("listbox")).toBeVisible();
  await chip.click();
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
  await expect.element(words).toHaveFocus();
  await new Promise((done) => setTimeout(done, 100));
  expect(bar().dataset.mode).toBe("focus");
});

test("a press on the words while the menu is open closes it and puts the cursor there", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("claude-code/opus"));
  const screen = await render(ModelBarStage, { profile: PROFILE });
  const words = screen.getByRole("textbox");
  await words.click();
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
  await expect.element(page.getByRole("listbox")).toBeVisible();
  await words.click();
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
  await expect.element(words).toHaveFocus();
});

test("a stored model of the retired hosted provider is never offered", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.choose.mockClear();
  const base = view("claude-code/opus");
  const hosted = entry("cloud", "claude-sonnet-5-5", "Claude Sonnet 5.5", true, ["lead"]);
  native.read.mockResolvedValue({ ...base, entries: [hosted, ...base.entries] });
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: Claude Code · Opus/u }).click();
  expect(page.getByRole("tab", { name: "Unavailable" }).query()).toBeNull();
  expect(page.getByRole("tab").all()[0]!.element().getAttribute("aria-label")).toBe("Claude Code");
  expect(native.choose).not.toHaveBeenCalled();
});
