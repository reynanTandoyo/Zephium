import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { surface } from "$domain/surface";
import { preferences } from "$domain/preferences";
import { emitNativeEvent } from "$shared/testing/native-events";
import ModeTabs from "../components/ModeTabs.svelte";

const native = vi.hoisted(() => ({
  run: vi.fn(async () => ({ accepted: true, operation_id: null })),
  settings: {} as Record<string, string>,
}));

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    runCommand: native.run,
    settingGet: async (key: string) => native.settings[key] ?? null,
  });
});

afterEach(() => {
  surface.dispose();
  preferences.dispose();
  native.settings = {};
});

test("the lit half follows native confirmation rather than command admission", async () => {
  await surface.init();
  const screen = await render(ModeTabs);
  const browse = screen.getByRole("radio", { name: "Browse", exact: true });
  const work = screen.getByRole("radio", { name: "Work", exact: true });

  await expect.element(browse).toHaveAttribute("aria-checked", "true");

  await work.click();
  expect(native.run).toHaveBeenCalledWith("browser.work");
  // Admission is not arrival: the thumb stays on Browse until native says so.
  await expect.element(work).toHaveAttribute("aria-checked", "false");
  emitNativeEvent("uiCommand", "browser.work");
  await expect.element(work).toHaveAttribute("aria-checked", "true");
  await expect.element(browse).toHaveAttribute("aria-checked", "false");

  await browse.click();
  expect(native.run).toHaveBeenCalledWith("browser.return");
  emitNativeEvent("uiCommand", "browser.return");
  await expect.element(browse).toHaveAttribute("aria-checked", "true");
});

test("re-selecting the side already showing asks native for nothing", async () => {
  await surface.init();
  const screen = await render(ModeTabs);
  native.run.mockClear();

  await screen.getByRole("radio", { name: "Browse", exact: true }).click();
  expect(native.run).not.toHaveBeenCalled();
});

test("the rail's switch marks where you are and goes to the other in one click", async () => {
  await surface.init();
  const screen = await render(ModeTabs, { compact: true });
  const browse = screen.getByRole("button", { name: "Browse", exact: true });
  const work = screen.getByRole("button", { name: "Work", exact: true });
  await expect.element(browse).toHaveAttribute("aria-pressed", "true");
  await work.click();
  expect(native.run).toHaveBeenCalledWith("browser.work");
  // The thumb moves only once native has moved there.
  await expect.element(work).toHaveAttribute("aria-pressed", "false");
  emitNativeEvent("uiCommand", "browser.work");
  await expect.element(work).toHaveAttribute("aria-pressed", "true");
});

test("with every other mode off the column has no switch at either width, and it comes back on", async () => {
  native.settings["work.enabled"] = "false";
  native.settings["mail.enabled"] = "false";
  native.settings["calendar.enabled"] = "false";
  await preferences.init();
  await surface.init();
  const wide = await render(ModeTabs);
  const rail = await render(ModeTabs, { compact: true, standalone: true });
  await expect.element(wide.getByRole("radio", { name: "Work" })).not.toBeInTheDocument();
  await expect.element(rail.getByRole("button", { name: "Work" })).not.toBeInTheDocument();
  expect(wide.container.textContent?.trim()).toBe("");

  emitNativeEvent("uiCommand", "preference.work.enabled=true");
  await expect.element(wide.getByRole("radio", { name: "Work", exact: true })).toBeVisible();
  await expect.element(rail.getByRole("button", { name: "Work", exact: true })).toBeVisible();
});

test("Mail and Calendar sit beside Browse and Work and each asks for its own page", async () => {
  await surface.init();
  const screen = await render(ModeTabs);
  const names = ["Browse", "Work", "Mail", "Calendar"];
  for (const name of names)
    await expect.element(screen.getByRole("radio", { name, exact: true })).toBeVisible();
  for (const [name, command] of [
    ["Mail", "browser.mail"],
    ["Calendar", "browser.calendar"],
  ] as const) {
    await screen.getByRole("radio", { name, exact: true }).click();
    expect(native.run).toHaveBeenLastCalledWith(command);
    emitNativeEvent("uiCommand", command);
    await expect
      .element(screen.getByRole("radio", { name, exact: true }))
      .toHaveAttribute("aria-checked", "true");
  }
});

test("the rail's switch has one cell per enabled mode and its thumb follows the mode", async () => {
  await surface.init();
  const screen = await render(ModeTabs, { compact: true });
  const group = screen.getByRole("group").element() as HTMLElement;
  expect(group.style.getPropertyValue("--count")).toBe("4");
  emitNativeEvent("uiCommand", "browser.calendar");
  await expect.poll(() => group.style.getPropertyValue("--index")).toBe("3");
});

test("a mode whose feature is off leaves the switch, and the switch ends below two modes", async () => {
  native.settings["mail.enabled"] = "false";
  await preferences.init();
  await surface.init();
  const screen = await render(ModeTabs);
  await expect.element(screen.getByRole("radio", { name: "Mail" })).not.toBeInTheDocument();
  await expect.element(screen.getByRole("radio", { name: "Calendar" })).toBeVisible();
  emitNativeEvent("uiCommand", "preference.work.enabled=false");
  emitNativeEvent("uiCommand", "preference.calendar.enabled=false");
  await expect.element(screen.getByRole("radio", { name: "Browse" })).not.toBeInTheDocument();
});
