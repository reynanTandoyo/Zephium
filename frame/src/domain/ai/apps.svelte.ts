import { commands } from "$shared/ipc/bindings";
import type { WorkAiAppIdV1, WorkAiAppV1 } from "$shared/ipc/bindings";
import { observe } from "$shared/lib/observe";

const TIMEOUT = 9000;
/** A rescan runs each app to ask its version and sign-in. */
const SCAN_TIMEOUT = 30000;

/**
 * The AI apps on this PC: Claude Code, Antigravity and OpenCode. Rust scans
 * for them and runs their installers and sign-ins in a visible terminal; this
 * mirrors its last answer.
 */
export class AiApps {
  /** The scan so far; null until the first answer. */
  apps = $state.raw<readonly WorkAiAppV1[] | null>(null);
  /** A scan is running. */
  scanning = $state(false);
  /** Scanning failed, so nothing is known. */
  unavailable = $state(false);
  /** The app whose install or sign-in is being started. */
  busy = $state<WorkAiAppIdV1 | null>(null);
  /** The app whose last install or sign-in could not be started. */
  failed = $state<WorkAiAppIdV1 | null>(null);
  /** An install or sign-in was started, so coming back to the window checks again. */
  private awaiting = false;
  private generation = 0;
  private lifetime = new AbortController();

  /** `rescan` asks the apps again; otherwise Rust answers from its last scan. */
  async scan(rescan: boolean) {
    if (this.scanning) return;
    this.scanning = true;
    const generation = this.generation;
    const response = await observe(
      Promise.resolve().then(() => commands.workAiApps(rescan)),
      rescan ? SCAN_TIMEOUT : TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation) return;
    this.scanning = false;
    if (response.state !== "received") {
      this.unavailable = !this.apps;
      return;
    }
    this.unavailable = false;
    this.apps = response.value.apps;
    this.awaiting = false;
  }

  /** The window came back after an install or sign-in was started: check again. */
  returned() {
    if (this.awaiting) void this.scan(true);
  }

  install(app: WorkAiAppIdV1) {
    return this.start(app, () => commands.workAiAppInstall(app));
  }

  signIn(app: WorkAiAppIdV1) {
    return this.start(app, () => commands.workAiAppSignIn(app));
  }

  private async start(
    app: WorkAiAppIdV1,
    request: () => Promise<{ status: "ok" } | { status: "error" }>,
  ) {
    if (this.busy) return false;
    this.busy = app;
    this.failed = null;
    const generation = this.generation;
    const response = await observe(Promise.resolve().then(request), TIMEOUT, this.lifetime.signal);
    if (generation !== this.generation) return false;
    this.busy = null;
    if (response.state !== "received" || response.value.status !== "ok") {
      this.failed = app;
      return false;
    }
    this.awaiting = true;
    return true;
  }

  dispose() {
    this.generation++;
    this.lifetime.abort();
    this.lifetime = new AbortController();
    this.apps = null;
    this.scanning = false;
    this.unavailable = false;
    this.busy = null;
    this.failed = null;
    this.awaiting = false;
  }
}
