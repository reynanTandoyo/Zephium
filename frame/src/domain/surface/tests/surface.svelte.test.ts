import { beforeEach, describe, expect, it, vi } from "vitest";
const harness = vi.hoisted(() => ({
  listener: null as null | ((event: { payload: string }) => void),
  run: vi.fn(),
  wait: vi.fn(),
}));
vi.mock("svelte", () => ({ flushSync: (callback: () => void) => callback() }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ runCommand: harness.run });
});
vi.mock("$domain/operations/operations", () => ({ waitForDisposition: harness.wait }));
vi.mock("$shared/ipc/native-events", () => ({
  events: {
    browserReturn: { listen: () => Promise.resolve(() => {}) },
    uiCommand: {
      listen: (listener: typeof harness.listener) => {
        harness.listener = listener;
        return Promise.resolve(() => {});
      },
    },
  },
}));
describe("browser destination failures", () => {
  beforeEach(() => {
    vi.resetModules();
    harness.listener = null;
    harness.run.mockReset();
    harness.wait.mockReset();
  });
  it("clears an earlier rejection when the native menu confirms Settings", async () => {
    const page = await import("../surface.svelte");
    await page.init();
    harness.run.mockResolvedValue({ accepted: false, operation_id: null });
    await page.open("settings");
    expect(page.navigationFailed()).toBe(true);
    harness.listener?.({ payload: "browser.settings" });
    expect(page.currentPage()).toBe("settings");
    expect(page.navigationFailed()).toBe(false);
    page.dispose();
  });
  it("does not replace a newer successful navigation with an old rejection", async () => {
    const page = await import("../surface.svelte");
    await page.init();
    let reject!: (error: Error) => void;
    harness.run.mockReturnValue(
      new Promise((_resolve, failure) => {
        reject = failure;
      }),
    );
    const pending = page.open("settings");
    harness.listener?.({ payload: "browser.settings" });
    reject(new Error("old request failed"));
    await pending;
    expect(page.currentPage()).toBe("settings");
    expect(page.navigationFailed()).toBe(false);
    page.dispose();
  });
});

it("shares initialization and does not report a disposed navigation as a new failure", async () => {
  const page = await import("../surface.svelte");
  const initializing = page.init();
  expect(page.init()).toBe(initializing);
  await initializing;
  harness.run.mockImplementation(() => new Promise(() => {}));
  const navigation = page.open("settings");
  page.dispose();
  await navigation;
  expect(page.currentPage()).toBeNull();
  expect(page.navigationFailed()).toBe(false);
});

describe("closing a page", () => {
  beforeEach(() => {
    vi.resetModules();
    harness.listener = null;
    harness.run.mockReset();
    harness.run.mockResolvedValue({ accepted: true, operation_id: null });
  });
  it("returns to Work when the page was opened over it, and to Browse otherwise", async () => {
    const page = await import("../surface.svelte");
    await page.init();
    harness.listener?.({ payload: "browser.work" });
    harness.listener?.({ payload: "browser.settings" });
    await page.close();
    expect(harness.run).toHaveBeenLastCalledWith("browser.work");
    harness.listener?.({ payload: "browser.work" });
    harness.listener?.({ payload: "browser.return" });
    harness.listener?.({ payload: "browser.settings" });
    await page.close();
    expect(harness.run).toHaveBeenLastCalledWith("browser.return");
    page.dispose();
  });
});
describe("full-window modes", () => {
  beforeEach(() => {
    vi.resetModules();
    harness.listener = null;
    harness.run.mockReset();
    harness.wait.mockReset();
  });
  it("asks for Mail and Calendar by their commands and shows them once native confirms", async () => {
    const page = await import("../surface.svelte");
    await page.init();
    harness.run.mockResolvedValue({ accepted: true, operation_id: null });
    for (const mode of ["mail", "calendar"] as const) {
      await page.open(mode);
      expect(harness.run).toHaveBeenLastCalledWith(`browser.${mode}`);
      // Admission is not arrival.
      expect(page.currentPage()).not.toBe(mode);
      harness.listener?.({ payload: `browser.${mode}` });
      expect(page.currentPage()).toBe(mode);
    }
    page.dispose();
  });
  it("closing Mail returns to Browse, and Settings opened over it returns to it", async () => {
    const page = await import("../surface.svelte");
    await page.init();
    harness.run.mockResolvedValue({ accepted: true, operation_id: null });
    harness.listener?.({ payload: "browser.mail" });
    harness.listener?.({ payload: "browser.settings" });
    await page.close();
    expect(harness.run).toHaveBeenLastCalledWith("browser.mail");
    harness.listener?.({ payload: "browser.mail" });
    await page.close();
    expect(harness.run).toHaveBeenLastCalledWith("browser.return");
    page.dispose();
  });
  it("lists Browse first and hides a mode whose feature is off", async () => {
    const { MODES, enabledModes, modeOf, isModePage } = await import("../modes");
    expect(MODES.map((mode) => mode.id)).toEqual(["browse", "work", "mail", "calendar"]);
    expect(enabledModes(() => true).map((mode) => mode.id)).toEqual([
      "browse",
      "work",
      "mail",
      "calendar",
    ]);
    expect(enabledModes((feature) => feature !== "mail").map((mode) => mode.id)).toEqual([
      "browse",
      "work",
      "calendar",
    ]);
    expect(enabledModes(() => false).map((mode) => mode.id)).toEqual(["browse"]);
    expect(modeOf(null)).toBe("browse");
    expect(modeOf("settings")).toBe("browse");
    expect(modeOf("calendar")).toBe("calendar");
    expect(isModePage("history")).toBe(false);
  });
});
