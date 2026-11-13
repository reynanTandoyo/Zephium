import "$styles/global.css";
import { beforeEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import type {
  WorkAiAppStateV1,
  WorkAiAppsV1,
  WorkLocalServerV1,
  WorkMoreModelsV1_Serialize,
} from "$shared/ipc/bindings";
import { PROFILE, aiApp, models } from "$shared/testing/work-models";
import { emitNativeEvent } from "$shared/testing/native-events";
import AiPage from "../components/sections/AiPage.svelte";

vi.mock("$domain/tabs", () => ({
  tabs: { profile: () => ({ id: "00000000000000000000000001", kind: "regular" }) },
}));
const native = vi.hoisted(() => ({
  read: vi.fn(),
  set: vi.fn(),
  test: vi.fn(),
  choose: vi.fn(),
  scan: vi.fn(),
  install: vi.fn(),
  signIn: vi.fn(),
  local: vi.fn(async () => [] as WorkLocalServerV1[]),
  endpoint: vi.fn(),
  more: vi.fn(async (): Promise<WorkMoreModelsV1_Serialize> => ({
    provider: "compatible",
    entries: [],
    fault: null,
  })),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    workModels: native.read,
    workSetProviderKey: native.set,
    workTestProviderKey: native.test,
    workChooseModel: native.choose,
    workAiApps: native.scan,
    workAiAppInstall: native.install,
    workAiAppSignIn: native.signIn,
    workLocalServers: native.local,
    workSetModelEndpoint: native.endpoint,
    workMoreModels: native.more,
    voiceStatus: async () => ({
      version: 1,
      engines: ["on_device", "open_ai"],
      chosen: null,
      effective: "on_device",
      language: null,
      languages: ["en-US"],
      on_device: "en_US",
      microphone: "granted",
    }),
    workDecisionPreference: async () => ({
      version: 1,
      profile: PROFILE,
      choice: "recommended",
      effective: "standard",
      typesafe_key_present: false,
      error: null,
    }),
  });
});

const apps = (
  states: Partial<Record<"claude_code" | "antigravity" | "open_code", WorkAiAppStateV1>>,
): WorkAiAppsV1 => ({
  apps: (["claude_code", "antigravity", "open_code"] as const).map((app) =>
    aiApp(app, states[app] ?? "missing"),
  ),
});

beforeEach(() => {
  native.scan.mockReset().mockResolvedValue(apps({}));
});

test("each AI app on this PC shows its state, and the scan runs when the page opens", async () => {
  await page.viewport(1100, 1400);
  native.read.mockResolvedValue(models({ apps: { claude_code: "ready" } }));
  native.scan.mockResolvedValue(
    apps({ claude_code: "ready", antigravity: "sign_in_needed", open_code: "missing" }),
  );
  const screen = await render(AiPage);
  await expect.element(screen.getByRole("heading", { name: "AI apps on this PC" })).toBeVisible();
  await expect.element(screen.getByText("Installed · signed in")).toBeVisible();
  await expect.element(screen.getByText("1.0.0")).toBeVisible();
  await expect.element(screen.getByText("Installed · sign in needed")).toBeVisible();
  await expect.element(screen.getByText("Not installed")).toBeVisible();
  expect(native.scan).toHaveBeenCalledWith(false);
  // Only the apps and the person's own endpoint are offered.
  await expect.element(screen.getByRole("heading", { name: "Your own endpoint" })).toBeVisible();
  for (const name of ["Anthropic", "OpenAI", "Google", "DeepSeek", "OpenRouter", "xAI"])
    await expect.element(screen.getByText(name, { exact: true })).not.toBeInTheDocument();
});

test("an app that needs a sign-in opens its sign-in", async () => {
  await page.viewport(1100, 1400);
  native.read.mockResolvedValue(models({ apps: { antigravity: "sign_in_needed" } }));
  native.scan.mockResolvedValue(apps({ antigravity: "sign_in_needed" }));
  native.signIn.mockResolvedValue({ status: "ok", data: null });
  const screen = await render(AiPage);
  await screen.getByRole("button", { name: "Sign in" }).click();
  expect(native.signIn).toHaveBeenCalledWith("antigravity");
});

test("a missing app is installed only after its command is shown and confirmed", async () => {
  await page.viewport(1100, 1400);
  native.read.mockResolvedValue(models({}));
  native.scan.mockResolvedValue(apps({}));
  native.install.mockReset().mockResolvedValue({ status: "ok", data: null });
  const screen = await render(AiPage);
  const installs = screen.getByRole("button", { name: "Install", exact: true });
  await installs.first().click();
  const confirm = screen.getByRole("group", { name: "Install Claude Code?" });
  await expect.element(confirm).toBeVisible();
  await expect.element(confirm.getByText("irm https://claude.ai/install.ps1 | iex")).toBeVisible();
  expect(native.install).not.toHaveBeenCalled();
  await confirm.getByRole("button", { name: "Cancel" }).click();
  await expect.element(confirm).not.toBeInTheDocument();
  expect(native.install).not.toHaveBeenCalled();

  await installs.first().click();
  await confirm.getByRole("button", { name: "Install" }).click();
  expect(native.install).toHaveBeenCalledWith("claude_code");
});

test("an app with no install command opens its download page instead", async () => {
  await page.viewport(1100, 1400);
  native.read.mockResolvedValue(models({}));
  native.scan.mockResolvedValue(apps({}));
  native.install.mockReset().mockResolvedValue({ status: "ok", data: null });
  const screen = await render(AiPage);
  await screen.getByRole("button", { name: "Install", exact: true }).nth(1).click();
  const confirm = screen.getByRole("group", { name: "Install Antigravity?" });
  await expect.element(confirm).toHaveTextContent("download page");
  await confirm.getByRole("button", { name: "Install" }).click();
  expect(native.install).toHaveBeenCalledWith("antigravity");
});

test("Check again scans the apps again and shows what changed", async () => {
  await page.viewport(1100, 1400);
  native.read.mockResolvedValue(models({}));
  native.scan
    .mockReset()
    .mockResolvedValueOnce(apps({}))
    .mockResolvedValueOnce(apps({ claude_code: "ready" }));
  const screen = await render(AiPage);
  await expect.element(screen.getByText("Not installed").first()).toBeVisible();
  await screen.getByRole("button", { name: "Check again" }).click();
  expect(native.scan).toHaveBeenLastCalledWith(true);
  await expect.element(screen.getByText("Installed · signed in")).toBeVisible();
});

test("an app that could not be checked says so", async () => {
  await page.viewport(1100, 1400);
  native.read.mockResolvedValue(models({}));
  native.scan.mockReset().mockRejectedValue(new Error("unavailable"));
  const screen = await render(AiPage);
  await expect.element(screen.getByText("Couldn’t check").first()).toBeVisible();
});

test("a refused endpoint key is explained and saving a working one closes the form", async () => {
  await page.viewport(1100, 1400);
  native.read.mockResolvedValue(models({}));
  native.scan.mockResolvedValue(apps({}));
  native.endpoint.mockResolvedValue(models({ base: "http://localhost:11434/v1" }));
  native.set
    .mockResolvedValueOnce({
      ...models({ base: "http://localhost:11434/v1" }),
      fault: "key_refused",
    })
    .mockResolvedValueOnce(
      models({ base: "http://localhost:11434/v1", keys: { compatible: "set" } }),
    );
  const screen = await render(AiPage);

  await screen.getByRole("button", { name: "Set up" }).click();
  await screen.getByLabelText("Server address").fill("http://localhost:11434/v1");
  await screen.getByLabelText("API key, if the server needs one").fill("wrong");
  await screen.getByRole("button", { name: "Save" }).click();
  expect(native.set).toHaveBeenCalledWith(PROFILE, "compatible", "wrong");
  await expect.element(screen.getByText("Custom server didn’t accept this key.")).toBeVisible();

  await screen.getByLabelText("API key, if the server needs one").fill("right");
  await screen.getByRole("button", { name: "Save" }).click();
  await expect.element(screen.getByRole("button", { name: "Save" })).not.toBeInTheDocument();
});

test("a role follows Automatic until the person chooses a model", async () => {
  await page.viewport(1100, 1400);
  native.scan.mockResolvedValue(apps({ claude_code: "ready" }));
  native.read.mockResolvedValue(
    models({ apps: { claude_code: "ready" }, lead: "claude-code/opus" }),
  );
  native.choose.mockResolvedValue(
    models({
      apps: { claude_code: "ready" },
      lead: "claude-code/sonnet",
      chosen: "claude-code/sonnet",
    }),
  );
  const screen = await render(AiPage);
  await expect.element(screen.getByText("Automatic · Claude Code · Opus")).toBeVisible();
  await screen.getByText("Automatic · Claude Code · Opus").click();
  await page.getByRole("option", { name: /Claude Code · Sonnet/ }).click();
  expect(native.choose).toHaveBeenCalledWith(PROFILE, "lead", "claude-code/sonnet");
});

test("a refused model change restores the stored choice and reports the failure", async () => {
  await page.viewport(1100, 1400);
  native.scan.mockResolvedValue(apps({ claude_code: "ready" }));
  const view = models({ apps: { claude_code: "ready" }, lead: "claude-code/opus" });
  native.read.mockResolvedValue(view);
  native.choose.mockReset().mockResolvedValue({ ...view, fault: "request" });
  const screen = await render(AiPage);
  const control = screen.getByLabelText("Work", { exact: true });
  await control.click();
  await page.getByRole("option", { name: /Claude Code · Sonnet/u }).click();
  await expect
    .element(screen.getByRole("alert"))
    .toHaveTextContent("The model couldn’t be changed");
  await expect.element(control).toHaveTextContent("Automatic · Claude Code · Opus");
});

test("billing trouble with the custom endpoint is explained", async () => {
  await page.viewport(1100, 1400);
  native.scan.mockResolvedValue(apps({}));
  const view = models({ base: "http://localhost:11434/v1" });
  view.providers = view.providers.map((provider) =>
    provider.provider === "compatible" ? { ...provider, fault: "billing" } : provider,
  );
  native.read.mockResolvedValue(view);
  const screen = await render(AiPage);
  await expect
    .element(
      screen.getByText("Check your provider’s billing or available credits, then try again."),
    )
    .toBeVisible();
  await expect.element(screen.getByText("Zephium Cloud", { exact: true })).not.toBeInTheDocument();
});

test("Settings AI exposes Jev setup beside the regular model controls with honest fallback", async () => {
  await page.viewport(1100, 1400);
  native.scan.mockResolvedValue(apps({ claude_code: "ready" }));
  native.read.mockResolvedValue(
    models({ apps: { claude_code: "ready" }, lead: "claude-code/sonnet" }),
  );
  const screen = await render(AiPage);
  await expect.element(screen.getByRole("button", { name: "Add Jev key" })).toBeVisible();
  await expect.element(screen.getByRole("radio", { name: "Recommended" })).toBeChecked();
  await expect.element(screen.getByText("Active: Standard")).toBeVisible();
  await expect.element(screen.getByRole("heading", { name: "Work", exact: true })).toBeVisible();
});

test("no hosted group or sign-in is offered, and nothing is set up yet", async () => {
  await page.viewport(1100, 1400);
  native.scan.mockResolvedValue(apps({}));
  native.read.mockResolvedValue(models({}));
  const screen = await render(AiPage);
  await expect
    .element(
      screen.getByText("Set up an AI app on this PC, or your own endpoint", { exact: false }),
    )
    .toBeVisible();
  await expect.element(screen.getByText("Zephium Cloud", { exact: true })).not.toBeInTheDocument();
  await expect.element(screen.getByRole("button", { name: "Sign in…" })).not.toBeInTheDocument();
});

test("a model server already running here is one click away", async () => {
  await page.viewport(1100, 1400);
  native.read.mockResolvedValue(models({}));
  native.local.mockResolvedValue([
    { kind: "ollama", base: "http://localhost:11434/v1", models: 3 },
  ]);
  native.endpoint.mockResolvedValue(models({ base: "http://localhost:11434/v1" }));
  const screen = await render(AiPage);
  await screen.getByRole("button", { name: "Use Ollama on this computer" }).click();
  expect(native.endpoint).toHaveBeenCalledWith(PROFILE, "http://localhost:11434/v1");
  await expect
    .element(screen.getByText("Set OLLAMA_CONTEXT_LENGTH", { exact: false }))
    .toBeVisible();
});

test("a custom server is asked for its models once per address, even when it has none", async () => {
  await page.viewport(1100, 1200);
  native.more.mockReset();
  native.read.mockResolvedValue(models({ base: "http://localhost:11434/v1" }));
  for (const answer of [
    { provider: "compatible", entries: [], fault: null },
    { provider: "compatible", entries: [], fault: "unreachable" },
  ] satisfies WorkMoreModelsV1_Serialize[]) {
    native.more.mockReset();
    native.more.mockResolvedValue(answer);
    const screen = await render(AiPage);
    await expect.element(screen.getByText("http://localhost:11434/v1")).toBeVisible();
    await new Promise((settle) => setTimeout(settle, 400));
    expect(native.more).toHaveBeenCalledTimes(1);
    emitNativeEvent("workModelsChanged", { version: 1 });
    await new Promise((settle) => setTimeout(settle, 200));
    expect(native.more).toHaveBeenCalledTimes(1);
    screen.unmount();
  }
  native.more.mockReset();
  native.more.mockResolvedValue({ provider: "compatible", entries: [], fault: null });
  const screen = await render(AiPage);
  await expect.poll(() => native.more.mock.calls.length).toBe(1);
  native.read.mockResolvedValue(models({ base: "http://localhost:1234/v1" }));
  emitNativeEvent("workModelsChanged", { version: 1 });
  await expect.poll(() => native.more.mock.calls.length).toBe(2);
  screen.unmount();
});
