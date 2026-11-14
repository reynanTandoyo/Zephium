import type { MailtoV1 } from "$shared/ipc/bindings";
import { events } from "$shared/ipc/native-events";

/**
 * A clicked `mailto:` link, waiting for Mail to open it. Rust parses and
 * bounds the link and only announces it when Mail is on and an account
 * exists; the Mail page takes it once its accounts are read. Reference
 * counted like the unread count: the first holder listens, the last stops.
 */
export class MailtoInbox {
  /** The link Mail has not opened yet. */
  pending = $state.raw<MailtoV1 | null>(null);
  private holders = 0;
  private stop: (() => void) | null = null;
  private generation = 0;

  hold(): () => void {
    this.holders++;
    if (this.holders === 1) void this.start();
    let held = true;
    return () => {
      if (!held) return;
      held = false;
      this.holders--;
      if (this.holders === 0) this.halt();
    };
  }

  /** Takes the waiting link, leaving none. */
  take(): MailtoV1 | null {
    const link = this.pending;
    this.pending = null;
    return link;
  }

  receive(link: MailtoV1) {
    this.pending = link;
  }

  private async start() {
    const generation = ++this.generation;
    const stop = await events.mailCompose.listen((event) => this.receive(event.payload.mailto));
    if (generation !== this.generation) {
      stop();
      return;
    }
    this.stop = stop;
  }

  private halt() {
    this.generation++;
    this.stop?.();
    this.stop = null;
    this.pending = null;
  }
}

/** The app's one inbox for `mailto:` links. */
export const mailtoInbox = new MailtoInbox();
