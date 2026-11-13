import { afterEach, expect, test, vi } from "vitest";
import { cleanup, render } from "vitest-browser-svelte";
import { surface } from "$domain/surface";
import { preferences } from "$domain/preferences";
import { emitNativeEvent } from "$shared/testing/native-events";
import ModeTabs from "../components/ModeTabs.svelte";

const native = vi.hoisted(() => ({
  settings: {} as Record<string, string>,
  unread: 0,
  alerts: [] as Array<[string, string]>,
}));

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    runCommand: async () => ({ accepted: true, operation_id: null }),
    settingGet: async (key: string) => native.settings[key] ?? null,
    // Only the id is read here; the rest of the account record is not needed.
    mailAccounts: async () => ({ status: "ok", data: [{ id: "a1" }] as never }),
    mailFolders: async () => ({
      status: "ok",
      data: [
        {
          path: "INBOX",
          name: "INBOX",
          delimiter: "/",
          role: "inbox",
          unread: native.unread,
          total: native.unread,
          selectable: true,
        },
      ],
    }),
    mailNotifyNew: async (title: string, body: string) => {
      native.alerts.push([title, body]);
    },
  });
});

afterEach(async () => {
  await cleanup();
  surface.dispose();
  preferences.dispose();
  native.settings = {};
  native.unread = 0;
  native.alerts = [];
});

/** The mail domain loads on first need, so the first read may take a moment. */
const LOAD = { timeout: 10_000 };

const badge = (container: Element) => container.querySelector(".badge")?.textContent?.trim();

test("the Mail segment carries the unread count of every inbox and names it", async () => {
  native.unread = 4;
  await surface.init();
  const screen = await render(ModeTabs);
  await expect.poll(() => badge(screen.container), LOAD).toBe("4");
  await expect
    .element(screen.getByText("Mail, 4 unread", { exact: true }), LOAD)
    .toBeInTheDocument();
  await expect.element(screen.getByRole("radio", { name: "Mail", exact: true })).toBeVisible();
});

test("no badge shows at zero unread", async () => {
  native.unread = 0;
  await surface.init();
  const screen = await render(ModeTabs);
  // Give the first read a chance to settle before asserting the absence.
  await expect.poll(() => screen.container.querySelector(".badge")).toBeNull();
  await expect.element(screen.getByRole("radio", { name: "Mail", exact: true })).toBeVisible();
});

test("counts past 99 show as 99+ and the name keeps the exact count", async () => {
  native.unread = 250;
  await surface.init();
  const screen = await render(ModeTabs);
  await expect.poll(() => badge(screen.container), LOAD).toBe("99+");
  await expect
    .element(screen.getByText("Mail, 250 unread", { exact: true }), LOAD)
    .toBeInTheDocument();
});

test("a mail change moves the count", async () => {
  native.unread = 2;
  await surface.init();
  const screen = await render(ModeTabs);
  await expect.poll(() => badge(screen.container), LOAD).toBe("2");

  native.unread = 5;
  emitNativeEvent("mailChanged", {
    account: "a1",
    kind: "new_mail",
    folder: "INBOX",
    unread: 5,
    new_messages: 3,
  });
  await expect.poll(() => badge(screen.container), LOAD).toBe("5");
});

test("the rail's Mail mark shows the count and is named with it", async () => {
  native.unread = 7;
  await surface.init();
  const screen = await render(ModeTabs, { compact: true });
  const mail = screen.getByRole("button", { name: "Mail, 7 unread", exact: true });
  await expect.element(mail, LOAD).toBeVisible();
  await expect
    .poll(() => mail.element().querySelector(".count")?.textContent?.trim(), LOAD)
    .toBe("7");
  await expect
    .element(screen.getByRole("button", { name: "Browse", exact: true }))
    .toHaveAttribute("aria-pressed", "true");
});

test("with Mail off there is no Mail segment and no count is read", async () => {
  native.settings["mail.enabled"] = "false";
  native.unread = 9;
  await preferences.init();
  await surface.init();
  const screen = await render(ModeTabs);
  await expect.element(screen.getByRole("radio", { name: "Mail" })).not.toBeInTheDocument();
  expect(screen.container.querySelector(".badge")).toBeNull();
});
