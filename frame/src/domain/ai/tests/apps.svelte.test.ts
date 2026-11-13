import { afterEach, expect, test, vi } from "vitest";
import type { WorkAiAppsV1 } from "$shared/ipc/bindings";
import { aiApp } from "$shared/testing/work-models";

const host = vi.hoisted(() => ({
  scan: vi.fn(),
  install: vi.fn(),
  signIn: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    workAiApps: host.scan,
    workAiAppInstall: host.install,
    workAiAppSignIn: host.signIn,
  });
});

const { AiApps } = await import("../apps.svelte");

const scan = (...states: Parameters<typeof aiApp>[1][]): WorkAiAppsV1 => ({
  apps: (["claude_code", "antigravity", "open_code"] as const).map((app, index) =>
    aiApp(app, states[index] ?? "missing"),
  ),
});

afterEach(() => {
  host.scan.mockReset();
  host.install.mockReset();
  host.signIn.mockReset();
});

test("the first scan asks for the cached answer and keeps what it says", async () => {
  host.scan.mockResolvedValue(scan("ready", "sign_in_needed"));
  const apps = new AiApps();
  expect(apps.apps).toBeNull();
  await apps.scan(false);
  expect(host.scan).toHaveBeenCalledWith(false);
  expect(apps.apps?.map((app) => app.state)).toEqual(["ready", "sign_in_needed", "missing"]);
  expect(apps.unavailable).toBe(false);
  expect(apps.scanning).toBe(false);
  apps.dispose();
});

test("a scan in progress is not started twice", async () => {
  let release!: (value: WorkAiAppsV1) => void;
  host.scan.mockReturnValue(new Promise((resolve) => (release = resolve)));
  const apps = new AiApps();
  const first = apps.scan(true);
  void apps.scan(true);
  expect(apps.scanning).toBe(true);
  await vi.waitFor(() => expect(host.scan).toHaveBeenCalledTimes(1));
  release(scan("ready"));
  await first;
  expect(apps.scanning).toBe(false);
  apps.dispose();
});

test("a failed first scan is unavailable, and a failed rescan keeps the last answer", async () => {
  host.scan.mockRejectedValueOnce(new Error("native"));
  const apps = new AiApps();
  await apps.scan(false);
  expect(apps.unavailable).toBe(true);
  expect(apps.apps).toBeNull();

  host.scan.mockResolvedValueOnce(scan("ready"));
  await apps.scan(true);
  expect(apps.unavailable).toBe(false);
  host.scan.mockRejectedValueOnce(new Error("native"));
  await apps.scan(true);
  expect(apps.unavailable).toBe(false);
  expect(apps.apps?.[0]?.state).toBe("ready");
  apps.dispose();
});

test("starting an install or a sign-in records a failure for that app only", async () => {
  host.install.mockResolvedValue({ status: "error", error: "unavailable" });
  host.signIn.mockResolvedValue({ status: "ok", data: null });
  const apps = new AiApps();
  expect(await apps.install("open_code")).toBe(false);
  expect(host.install).toHaveBeenCalledWith("open_code");
  expect(apps.failed).toBe("open_code");
  expect(apps.busy).toBeNull();

  expect(await apps.signIn("claude_code")).toBe(true);
  expect(host.signIn).toHaveBeenCalledWith("claude_code");
  expect(apps.failed).toBeNull();
  apps.dispose();
});

test("coming back to the window checks again only after an install or sign-in was started", async () => {
  host.scan.mockResolvedValue(scan("ready"));
  host.signIn.mockResolvedValue({ status: "ok", data: null });
  const apps = new AiApps();
  await apps.scan(false);
  apps.returned();
  expect(host.scan).toHaveBeenCalledTimes(1);

  await apps.signIn("antigravity");
  apps.returned();
  await vi.waitFor(() => expect(host.scan).toHaveBeenCalledTimes(2));
  expect(host.scan).toHaveBeenLastCalledWith(true);
  await vi.waitFor(() => expect(apps.scanning).toBe(false));
  // The check that followed settles the wait: a second return does not check again.
  apps.returned();
  expect(host.scan).toHaveBeenCalledTimes(2);
  apps.dispose();
});

test("an answer that arrives after dispose is dropped", async () => {
  let release!: (value: WorkAiAppsV1) => void;
  host.scan.mockReturnValue(new Promise((resolve) => (release = resolve)));
  const apps = new AiApps();
  const pending = apps.scan(false);
  apps.dispose();
  release(scan("ready"));
  await pending;
  expect(apps.apps).toBeNull();
});
