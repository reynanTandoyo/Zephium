import { commands } from "$shared/ipc/bindings";
import type { MailContactV1 } from "$shared/ipc/bindings";
import { callMail, READ_TIMEOUT } from "./call";
import { typedEntry } from "./address";

/** Contacts asked for per keystroke burst; the backend bounds it again. */
export const SUGGESTION_LIMIT = 8;
const DEFAULT_DELAY = 120;

/**
 * Suggestions for a recipient field while it is typed in. Rust answers from
 * the local mail cache only (no network), so this works offline. A newer
 * request outranks an older answer; a failed lookup simply shows nothing.
 */
export class ContactSuggester {
  results = $state.raw<readonly MailContactV1[]>([]);
  private sequence = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private lifetime = new AbortController();

  constructor(private readonly delay = DEFAULT_DELAY) {}

  /** Asks for the entry at the end of `text`; nothing is asked for an empty entry. */
  request(text: string) {
    clearTimeout(this.timer);
    const entry = typedEntry(text);
    if (!entry) {
      this.clear();
      return;
    }
    this.timer = setTimeout(() => void this.load(entry), this.delay);
  }

  private async load(entry: string) {
    const sequence = ++this.sequence;
    const outcome = await callMail(
      () => commands.mailContacts(entry, SUGGESTION_LIMIT),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (sequence !== this.sequence || outcome.ok === "aborted") return;
    this.results = outcome.ok === true ? outcome.value : [];
  }

  clear() {
    clearTimeout(this.timer);
    this.sequence++;
    this.results = [];
  }

  dispose() {
    this.clear();
    this.lifetime.abort();
    this.lifetime = new AbortController();
  }
}
