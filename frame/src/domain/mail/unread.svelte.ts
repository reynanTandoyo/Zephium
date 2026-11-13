import * as m from "$shared/i18n/messages";
import { commands } from "$shared/ipc/bindings";
import type { MailChanged, MailFolderV1 } from "$shared/ipc/bindings";
import { events } from "$shared/ipc/native-events";
import { preferences } from "$domain/preferences";
import { surface } from "$domain/surface";
import { callMail, READ_TIMEOUT } from "./call";

/** Counts above this show as the cap with a plus. */
const BADGE_CAP = 99;
/** How long arrivals gather before one alert goes up, so a burst is one toast. */
const ALERT_BATCH_MS = 1500;

/** The badge's text for a count: nothing at zero, `99+` past the cap. */
export function badgeText(count: number): string | null {
  if (!(count > 0)) return null;
  return count > BADGE_CAP ? `${BADGE_CAP}+` : String(count);
}

/** Unread messages in every inbox, over the folder lists given. */
export function unreadInInboxes(
  folders: Readonly<Record<string, readonly MailFolderV1[]>>,
): number {
  let total = 0;
  for (const list of Object.values(folders)) {
    for (const folder of list) {
      if (folder.role === "inbox") total += folder.unread;
    }
  }
  return total;
}

/** Only a new-mail change adds to a batch of arrivals. */
export function addArrivals(pending: number, change: MailChanged): number {
  return change.kind === "new_mail" ? pending + change.new_messages : pending;
}

/** The words of one alert for a batch: a count, never a sender, subject or body. */
export function newMailToast(count: number): { title: string; body: string } | null {
  if (!(count > 0)) return null;
  return {
    title: m.mail_toast_title(),
    body: count === 1 ? m.mail_toast_one() : m.mail_toast_many({ count }),
  };
}

type Deps = {
  /** True while the person is looking at Mail in a focused window. */
  inFront: () => boolean;
  alert: (title: string, body: string) => void;
  batchMs: number;
};

const nativeDeps: Deps = {
  inFront: () => document.hasFocus() && surface.modeOf(surface.currentPage()) === "mail",
  // A refused toast is not retried: the badge still shows the count.
  alert: (title, body) => {
    commands.mailNotifyNew(title, body).catch(() => {});
  },
  batchMs: ALERT_BATCH_MS,
};

/**
 * The unread count of every inbox, kept live from `zephium:mail-changed`, and
 * the new-mail alert that goes with it. Rust owns the counts; this reads them
 * back. Reference counted: the first holder starts it, the last one stops it.
 */
export class MailUnread {
  /** Unread messages in every inbox; zero while nothing holds it. */
  total = $state(0);

  private folders: Record<string, readonly MailFolderV1[]> = {};
  private holders = 0;
  private stop: (() => void) | null = null;
  private generation = 0;
  private lifetime = new AbortController();
  private pending = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private readonly deps: Deps;

  constructor(deps: Partial<Deps> = {}) {
    this.deps = { ...nativeDeps, ...deps };
  }

  /** Keeps the count live while a surface shows it. The result lets go once. */
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

  /** Called for each `zephium:mail-changed`; reads the affected folders again. */
  receive(change: MailChanged) {
    if (change.kind === "accounts") void this.load();
    else void this.refresh(change.account);
    this.pending = addArrivals(this.pending, change);
    if (this.pending > 0 && this.timer === null) {
      this.timer = setTimeout(() => this.flush(), this.deps.batchMs);
    }
  }

  private async start() {
    const generation = ++this.generation;
    this.lifetime = new AbortController();
    const stop = await events.mailChanged.listen((event) => this.receive(event.payload));
    if (generation !== this.generation) {
      stop();
      return;
    }
    this.stop = stop;
    await this.load();
  }

  private halt() {
    this.generation++;
    this.stop?.();
    this.stop = null;
    this.lifetime.abort();
    this.folders = {};
    this.total = 0;
    this.pending = 0;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  /** Every account's folders. A failed account keeps the count it had. */
  private async load() {
    const generation = this.generation;
    const accounts = await callMail(
      () => commands.mailAccounts(),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || accounts.ok !== true) return;
    const next: Record<string, readonly MailFolderV1[]> = {};
    await Promise.all(
      accounts.value.map(async (account) => {
        const outcome = await callMail(
          () => commands.mailFolders(account.id),
          READ_TIMEOUT,
          this.lifetime.signal,
        );
        next[account.id] = outcome.ok === true ? outcome.value : (this.folders[account.id] ?? []);
      }),
    );
    if (generation !== this.generation) return;
    this.folders = next;
    this.total = unreadInInboxes(next);
  }

  private async refresh(account: string) {
    const generation = this.generation;
    const outcome = await callMail(
      () => commands.mailFolders(account),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || outcome.ok !== true) return;
    this.folders = { ...this.folders, [account]: outcome.value };
    this.total = unreadInInboxes(this.folders);
  }

  /** One alert for what arrived in the batch, unless the person is already on Mail. */
  private flush() {
    this.timer = null;
    const count = this.pending;
    this.pending = 0;
    if (count <= 0 || this.deps.inFront()) return;
    if (preferences.value("mail.notifications") === "false") return;
    const toast = newMailToast(count);
    if (toast) this.deps.alert(toast.title, toast.body);
  }
}

/** The app's one count, shared by the mode switch and anything else that shows it. */
export const mailUnread = new MailUnread();
