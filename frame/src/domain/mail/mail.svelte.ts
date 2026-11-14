import { commands } from "$shared/ipc/bindings";
import type {
  MailAccountUpdateV1,
  MailAccountV1,
  MailBodyV1,
  MailChanged,
  MailFlagV1,
  MailFolderRoleV1,
  MailFolderV1,
  MailtoV1,
} from "$shared/ipc/bindings";
import { events } from "$shared/ipc/native-events";
import { callMail, IMAGES_TIMEOUT, READ_TIMEOUT, WRITE_TIMEOUT, type MailFailure } from "./call";
import { composeFor, withSignature, type ComposeMode } from "./compose";
import { ComposeSession } from "./compose.svelte";
import { buildThreads, rowKey, type MailRow } from "./thread";

const PAGE_SIZE = 50;
/** The combined view over every account's inbox. */
export const ALL_INBOXES = "*";

type Load = "idle" | "loading" | "ready" | "failed";

type MailSelection = { account: string; folder: string };

/** The remote images of the open message: fetched by the backend on request. */
type RemoteImages = {
  state: Load;
  /** Address as the message wrote it → `data:` URL. */
  images: Readonly<Record<string, string>>;
  error: MailFailure | null;
};

type OpenMessage = {
  row: MailRow;
  state: Load;
  body: MailBodyV1 | null;
  error: MailFailure | null;
  remote: RemoteImages;
};

/** At most this many images are requested per message (the backend's bound). */
const MAX_REMOTE_IMAGES = 24;

const noRemote = (): RemoteImages => ({ state: "idle", images: {}, error: null });

/** An action that could not be completed, shown until dismissed. */
type MailNotice = {
  action: "flag" | "move" | "delete" | "archive" | "junk" | "sync" | "open" | "account" | "order";
  error: MailFailure | "no_folder";
};

const ROLE_ORDER: Record<MailFolderRoleV1, number> = {
  inbox: 0,
  drafts: 1,
  sent: 2,
  archive: 3,
  junk: 4,
  trash: 5,
  other: 6,
};

/** Special-use folders first, in a fixed order, then the rest by name. */
function orderFolders(folders: readonly MailFolderV1[]): MailFolderV1[] {
  return folders
    .filter((folder) => folder.selectable)
    .toSorted(
      (a, b) =>
        ROLE_ORDER[a.role] - ROLE_ORDER[b.role] ||
        a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
    );
}

/** Failures that mean the server could not be reached, not that it said no. */
const UNREACHABLE: readonly MailFailure[] = ["offline", "timeout", "tls", "unavailable"];
const unreachable = (error: MailFailure | null) => error !== null && UNREACHABLE.includes(error);

const rowDate = (row: MailRow) => row.message.date ?? 0;
const byNewest = (a: MailRow, b: MailRow) =>
  rowDate(b) - rowDate(a) || b.message.uid - a.message.uid;

/**
 * Mail as the frame sees it. Rust owns accounts, the cache and every server
 * effect; this mirrors what it last said and records what is in flight.
 * Nothing is shown as done before the native reply says so.
 */
export class MailStore {
  accounts = $state.raw<readonly MailAccountV1[]>([]);
  accountsState = $state<Load>("idle");
  folders = $state.raw<Readonly<Record<string, readonly MailFolderV1[]>>>({});
  /** Accounts whose folder list could not be read. */
  folderFailures = $state.raw<readonly string[]>([]);
  selection = $state<MailSelection>({ account: ALL_INBOXES, folder: "INBOX" });
  query = $state("");
  rows = $state.raw<readonly MailRow[]>([]);
  listState = $state<Load>("idle");
  listError = $state<MailFailure | null>(null);
  /** The list is the local cache because the server could not be reached. */
  fromCache = $state(false);
  /** The last request that needed the server could not reach it. */
  private cut = $state(false);
  /**
   * Mail is showing what is saved on this computer: the list came from the
   * cache, or the last server request found no connection. Reading what is
   * cached still works; anything that needs the server says why it failed.
   */
  offline = $derived(this.fromCache || this.cut);
  loadingMore = $state(false);
  syncing = $state(false);
  open = $state<OpenMessage | null>(null);
  compose = $state<ComposeSession | null>(null);
  notice = $state<MailNotice | null>(null);
  /** Rows with an action in flight; they render as pending, unchanged. */
  pending = $state.raw<readonly string[]>([]);

  /** `next_before` per `account/folder` source; null means the end was reached. */
  private cursors = $state.raw<Readonly<Record<string, number | null>>>({});

  threads = $derived(buildThreads(this.rows));
  /** Unread in every inbox: the figure for a badge. */
  unread = $derived(
    Object.values(this.folders)
      .flatMap((list) => list.filter((folder) => folder.role === "inbox"))
      .reduce((sum, folder) => sum + folder.unread, 0),
  );
  hasMore = $derived(Object.values(this.cursors).some((cursor) => cursor !== null));
  /** Search is active for the current list. */
  searching = $derived(this.query.trim().length > 0);

  private stop: (() => void) | null = null;
  private active = false;
  private generation = 0;
  private listSequence = 0;
  private openSequence = 0;
  private refreshQueued = false;
  private lifetime = new AbortController();

  /** Subscribes before the first read so a change in between is not lost. */
  async start() {
    if (this.active) return;
    this.active = true;
    const generation = ++this.generation;
    this.lifetime = new AbortController();
    const stop = await events.mailChanged.listen((event) => this.changed(event.payload));
    if (!this.active || generation !== this.generation) {
      stop();
      return;
    }
    this.stop = stop;
    await this.loadAccounts();
  }

  dispose() {
    this.active = false;
    this.generation++;
    this.stop?.();
    this.stop = null;
    this.lifetime.abort();
    this.lifetime = new AbortController();
    this.compose?.dispose();
    this.compose = null;
    this.rows = [];
    this.open = null;
    this.accounts = [];
    this.folders = {};
    this.accountsState = "idle";
    this.listState = "idle";
    this.pending = [];
  }

  account(id: string) {
    return this.accounts.find((account) => account.id === id);
  }

  foldersOf(id: string): readonly MailFolderV1[] {
    return orderFolders(this.folders[id] ?? []);
  }

  inboxOf(id: string) {
    return (this.folders[id] ?? []).find((folder) => folder.role === "inbox");
  }

  roleFolder(id: string, role: MailFolderRoleV1) {
    return (this.folders[id] ?? []).find((folder) => folder.role === role);
  }

  // Accounts and folders ------------------------------------------------------

  async loadAccounts() {
    const generation = this.generation;
    if (this.accountsState !== "ready") this.accountsState = "loading";
    const outcome = await callMail(
      () => commands.mailAccounts(),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || outcome.ok === "aborted") return;
    if (!outcome.ok) {
      this.accountsState = this.accountsState === "ready" ? "ready" : "failed";
      return;
    }
    this.accounts = outcome.value;
    this.accountsState = "ready";
    const known = outcome.value.map((account) => account.id);
    const kept = Object.entries(this.folders).filter(([id]) => known.includes(id));
    this.folders = Object.fromEntries(kept);
    if (this.selection.account !== ALL_INBOXES && !known.includes(this.selection.account)) {
      this.selection = { account: ALL_INBOXES, folder: "INBOX" };
    }
    await Promise.all(outcome.value.map((account) => this.loadFolders(account.id)));
    if (generation === this.generation && outcome.value.length) await this.loadList();
    else if (!outcome.value.length) this.rows = [];
  }

  async loadFolders(account: string, fresh = false) {
    const generation = this.generation;
    const outcome = await callMail(
      () => (fresh ? commands.mailSync(account) : commands.mailFolders(account)),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || outcome.ok === "aborted") return false;
    if (!outcome.ok) {
      if (!this.folderFailures.includes(account))
        this.folderFailures = [...this.folderFailures, account];
      if (unreachable(outcome.error)) this.cut = true;
      return false;
    }
    if (fresh) this.cut = false;
    this.folderFailures = this.folderFailures.filter((id) => id !== account);
    this.folders = { ...this.folders, [account]: outcome.value };
    return true;
  }

  /** Asks the servers for fresh folders and counts; the cache is shown meanwhile. */
  async sync() {
    if (this.syncing) return;
    this.syncing = true;
    const generation = this.generation;
    const results = await Promise.all(
      this.accounts.map((account) => this.loadFolders(account.id, true)),
    );
    if (generation !== this.generation) return;
    this.syncing = false;
    if (results.some((ok) => !ok)) this.notice = { action: "sync", error: "unavailable" };
    await this.loadList();
  }

  // Selection and list --------------------------------------------------------

  /** The `account/folder` pairs the current selection reads. */
  private sources(): { account: string; folder: string }[] {
    if (this.selection.account === ALL_INBOXES) {
      return this.accounts.flatMap((account) => {
        const inbox = this.inboxOf(account.id);
        return [{ account: account.id, folder: inbox?.path ?? "INBOX" }];
      });
    }
    return [{ account: this.selection.account, folder: this.selection.folder }];
  }

  select(account: string, folder: string) {
    this.selection = { account, folder };
    this.query = "";
    this.open = null;
    this.notice = null;
    return this.loadList();
  }

  /** A folder's title and whether it is the combined inbox, for the header. */
  get current(): { combined: boolean; folder: MailFolderV1 | null } {
    if (this.selection.account === ALL_INBOXES) return { combined: true, folder: null };
    const folder = (this.folders[this.selection.account] ?? []).find(
      (candidate) => candidate.path === this.selection.folder,
    );
    return { combined: false, folder: folder ?? null };
  }

  /** Reads the first page (or the search) of every source. */
  async loadList() {
    const sequence = ++this.listSequence;
    const generation = this.generation;
    const sources = this.sources();
    const query = this.query.trim();
    if (!sources.length) {
      this.rows = [];
      this.listState = "ready";
      return;
    }
    this.listState = this.rows.length ? this.listState : "loading";
    if (this.listState !== "ready") this.listState = "loading";
    const results = await Promise.all(
      sources.map(async (source) => {
        if (query) {
          const outcome = await callMail(
            () => commands.mailSearch(source.account, source.folder, query),
            READ_TIMEOUT,
            this.lifetime.signal,
          );
          return { source, outcome, page: null };
        }
        const outcome = await callMail(
          () =>
            commands.mailMessages(source.account, source.folder, {
              before_uid: null,
              limit: PAGE_SIZE,
            }),
          READ_TIMEOUT,
          this.lifetime.signal,
        );
        return { source, outcome, page: true };
      }),
    );
    // A newer request (a click, an event, a keystroke) outranks this answer.
    if (generation !== this.generation || sequence !== this.listSequence) return;
    const rows: MailRow[] = [];
    const cursors: Record<string, number | null> = {};
    let failure: MailFailure | null = null;
    let succeeded = 0;
    let cached = false;
    let live = false;
    for (const { source, outcome, page } of results) {
      if (outcome.ok === "aborted") return;
      if (!outcome.ok) {
        failure ??= outcome.error;
        continue;
      }
      succeeded += 1;
      const key = `${source.account}/${source.folder}`;
      if (page && "messages" in outcome.value) {
        cursors[key] = outcome.value.next_before;
        cached ||= outcome.value.from_cache;
        live ||= !outcome.value.from_cache;
        for (const message of outcome.value.messages) rows.push(this.rowOf(source, message));
      } else if (Array.isArray(outcome.value)) {
        cursors[key] = null;
        live = true;
        for (const message of outcome.value) rows.push(this.rowOf(source, message));
      }
    }
    this.cursors = cursors;
    this.fromCache = cached;
    if (live) this.cut = false;
    else if (unreachable(failure)) this.cut = true;
    this.rows = rows.toSorted(byNewest);
    if (succeeded === 0) {
      this.listState = "failed";
      this.listError = failure;
    } else {
      this.listState = "ready";
      this.listError = failure;
    }
  }

  private rowOf(source: { account: string; folder: string }, message: MailRow["message"]): MailRow {
    return {
      key: rowKey(source.account, source.folder, message.uid),
      account: source.account,
      folder: source.folder,
      message,
    };
  }

  /** Reads the next page of every source that has one. Search has no paging. */
  async loadMore() {
    if (this.loadingMore || this.searching || !this.hasMore || this.listState !== "ready") return;
    this.loadingMore = true;
    const generation = this.generation;
    const sequence = this.listSequence;
    const next = this.sources().filter(
      (source) => this.cursors[`${source.account}/${source.folder}`] != null,
    );
    const results = await Promise.all(
      next.map(async (source) => ({
        source,
        outcome: await callMail(
          () =>
            commands.mailMessages(source.account, source.folder, {
              before_uid: this.cursors[`${source.account}/${source.folder}`] ?? null,
              limit: PAGE_SIZE,
            }),
          READ_TIMEOUT,
          this.lifetime.signal,
        ),
      })),
    );
    this.loadingMore = false;
    if (generation !== this.generation || sequence !== this.listSequence) return;
    const rows = [...this.rows];
    const seen = rows.map((row) => row.key);
    const cursors = { ...this.cursors };
    for (const { source, outcome } of results) {
      if (outcome.ok === "aborted") return;
      if (!outcome.ok) {
        this.listError = outcome.error;
        continue;
      }
      cursors[`${source.account}/${source.folder}`] = outcome.value.next_before;
      for (const message of outcome.value.messages) {
        const row = this.rowOf(source, message);
        if (!seen.includes(row.key)) {
          seen.push(row.key);
          rows.push(row);
        }
      }
    }
    this.cursors = cursors;
    this.rows = rows.toSorted(byNewest);
  }

  /** Searches the current folder; an empty query returns to the folder. */
  search(query: string) {
    this.query = query;
    this.open = null;
    return this.loadList();
  }

  // Events --------------------------------------------------------------------

  /** Reads again what the event says moved; the event itself carries no content. */
  private changed(event: MailChanged) {
    if (!this.active) return;
    if (event.kind === "accounts") {
      void this.loadAccounts();
      return;
    }
    void this.loadFolders(event.account);
    if (this.searching) return;
    const here =
      this.selection.account === ALL_INBOXES
        ? this.inboxOf(event.account)?.path === event.folder || event.folder === null
        : this.selection.account === event.account &&
          (event.folder === null || event.folder === this.selection.folder);
    if (here && (event.kind === "new_mail" || event.kind === "removed")) this.queueRefresh();
  }

  /** One list refresh at a time; events that arrive meanwhile share the next. */
  private queueRefresh() {
    if (this.refreshQueued) return;
    this.refreshQueued = true;
    queueMicrotask(() => {
      this.refreshQueued = false;
      if (this.active) void this.loadList();
    });
  }

  // Reading -------------------------------------------------------------------

  async openRow(row: MailRow) {
    const sequence = ++this.openSequence;
    const generation = this.generation;
    this.open = { row, state: "loading", body: null, error: null, remote: noRemote() };
    const outcome = await callMail(
      () => commands.mailMessage(row.account, row.folder, row.message.uid),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (
      generation !== this.generation ||
      sequence !== this.openSequence ||
      outcome.ok === "aborted"
    )
      return;
    if (!outcome.ok) {
      this.open = { row, state: "failed", body: null, error: outcome.error, remote: noRemote() };
      if (unreachable(outcome.error)) this.cut = true;
      return;
    }
    this.open = { row, state: "ready", body: outcome.value, error: null, remote: noRemote() };
    // Reading marks the message seen on the server; the row follows the answer.
    if (!row.message.flags.seen) void this.setSeen([row], true);
  }

  close() {
    this.openSequence++;
    this.open = null;
  }

  /**
   * "Load images": asks the backend to fetch the `https:` images of the open
   * message. They come back as `data:` URLs; nothing remote is loaded here.
   * Returns whether the images arrived (some may still be left out).
   */
  async loadRemoteImages(urls: readonly string[]) {
    const current = this.open;
    if (!current || current.state !== "ready" || current.remote.state === "loading") return false;
    const { row } = current;
    const wanted = urls.filter((url, at) => urls.indexOf(url) === at).slice(0, MAX_REMOTE_IMAGES);
    if (!wanted.length) return false;
    const sequence = this.openSequence;
    const generation = this.generation;
    this.open = { ...current, remote: { ...current.remote, state: "loading", error: null } };
    const outcome = await callMail(
      () => commands.mailFetchRemoteImages(row.account, row.folder, row.message.uid, wanted),
      IMAGES_TIMEOUT,
      this.lifetime.signal,
    );
    // Only the message that asked may receive the answer.
    if (
      generation !== this.generation ||
      sequence !== this.openSequence ||
      outcome.ok === "aborted" ||
      this.open?.row.key !== row.key
    )
      return false;
    if (!outcome.ok) {
      this.open = { ...this.open, remote: { state: "failed", images: {}, error: outcome.error } };
      return false;
    }
    const images = Object.fromEntries(outcome.value.map((image) => [image.url, image.data_url]));
    this.open = { ...this.open, remote: { state: "ready", images, error: null } };
    return true;
  }

  async saveAttachment(row: MailRow, part: number) {
    const outcome = await callMail(
      () => commands.mailAttachment(row.account, row.folder, row.message.uid, part),
      WRITE_TIMEOUT,
      this.lifetime.signal,
    );
    if (outcome.ok === "aborted") return null;
    if (!outcome.ok) {
      this.failedAction("open", outcome.error);
      return null;
    }
    return outcome.value;
  }

  // Actions -------------------------------------------------------------------

  private markPending(rows: readonly MailRow[], on: boolean) {
    const keys = rows.map((row) => row.key);
    this.pending = on
      ? [...this.pending, ...keys.filter((key) => !this.pending.includes(key))]
      : this.pending.filter((key) => !keys.includes(key));
  }

  /** An action on this row is in flight. */
  isPending(key: string) {
    return this.pending.includes(key);
  }

  /** Groups rows by `account/folder`, since each command addresses one folder. */
  private grouped(rows: readonly MailRow[]) {
    const groups: { account: string; folder: string; rows: MailRow[] }[] = [];
    for (const row of rows) {
      let group = groups.find((g) => g.account === row.account && g.folder === row.folder);
      if (!group) {
        group = { account: row.account, folder: row.folder, rows: [] };
        groups.push(group);
      }
      group.rows.push(row);
    }
    return groups;
  }

  private patchRows(rows: readonly MailRow[], patch: (row: MailRow) => MailRow | null) {
    const keys = rows.map((row) => row.key);
    this.rows = this.rows.flatMap((row) => {
      if (!keys.includes(row.key)) return [row];
      const next = patch(row);
      return next ? [next] : [];
    });
    if (this.open && keys.includes(this.open.row.key)) {
      const next = patch(this.open.row);
      this.open = next ? { ...this.open, row: next } : null;
    }
  }

  /** Sets a flag on the server, then updates the rows the server confirmed. */
  async setFlag(rows: readonly MailRow[], flag: Exclude<MailFlagV1, "deleted">, on: boolean) {
    const action = "flag";
    if (!rows.length) return true;
    this.markPending(rows, true);
    let allOk = true;
    for (const group of this.grouped(rows)) {
      const outcome = await callMail(
        () =>
          commands.mailSetFlags(
            group.account,
            group.folder,
            group.rows.map((r) => r.message.uid),
            flag,
            on,
          ),
        WRITE_TIMEOUT,
        this.lifetime.signal,
      );
      if (outcome.ok === "aborted") return false;
      if (!outcome.ok) {
        allOk = false;
        this.failedAction(action, outcome.error);
        continue;
      }
      const key = flag === "seen" ? "seen" : "flagged";
      this.patchRows(group.rows, (row) => ({
        ...row,
        message: { ...row.message, flags: { ...row.message.flags, [key]: on } },
      }));
    }
    this.markPending(rows, false);
    return allOk;
  }

  setSeen(rows: readonly MailRow[], seen: boolean) {
    return this.setFlag(rows, "seen", seen);
  }

  setFlagged(rows: readonly MailRow[], flagged: boolean) {
    return this.setFlag(rows, "flagged", flagged);
  }

  /** Moves messages; rows leave the list only after the server accepted it. */
  async move(rows: readonly MailRow[], destination: string, action: MailNotice["action"] = "move") {
    if (!rows.length) return true;
    const movable = rows.filter((row) => row.folder !== destination);
    if (!movable.length) return true;
    this.markPending(movable, true);
    let allOk = true;
    for (const group of this.grouped(movable)) {
      const outcome = await callMail(
        () =>
          commands.mailMove(
            group.account,
            group.folder,
            group.rows.map((r) => r.message.uid),
            destination,
          ),
        WRITE_TIMEOUT,
        this.lifetime.signal,
      );
      if (outcome.ok === "aborted") return false;
      if (!outcome.ok) {
        allOk = false;
        this.failedAction(action, outcome.error);
        continue;
      }
      this.patchRows(group.rows, () => null);
    }
    this.markPending(movable, false);
    return allOk;
  }

  /** Moves to the account's folder with this role, or says there is none. */
  async moveToRole(rows: readonly MailRow[], role: "archive" | "junk") {
    const accounts = rows.map((row) => row.account).filter((id, at, all) => all.indexOf(id) === at);
    for (const id of accounts) {
      if (!this.roleFolder(id, role)) {
        this.notice = { action: role, error: "no_folder" };
        return false;
      }
    }
    let ok = true;
    for (const id of accounts) {
      const target = this.roleFolder(id, role)!;
      ok =
        (await this.move(
          rows.filter((row) => row.account === id),
          target.path,
          role,
        )) && ok;
    }
    return ok;
  }

  /** Into Trash, or for good when the messages are already in Trash. */
  async remove(rows: readonly MailRow[]) {
    if (!rows.length) return true;
    this.markPending(rows, true);
    let allOk = true;
    for (const group of this.grouped(rows)) {
      const outcome = await callMail(
        () =>
          commands.mailDelete(
            group.account,
            group.folder,
            group.rows.map((r) => r.message.uid),
          ),
        WRITE_TIMEOUT,
        this.lifetime.signal,
      );
      if (outcome.ok === "aborted") return false;
      if (!outcome.ok) {
        allOk = false;
        this.failedAction("delete", outcome.error);
        continue;
      }
      this.patchRows(group.rows, () => null);
    }
    this.markPending(rows, false);
    return allOk;
  }

  /** An action that needs the server failed; say so, and note a lost connection. */
  private failedAction(action: MailNotice["action"], error: MailFailure) {
    this.notice = { action, error };
    if (unreachable(error)) this.cut = true;
  }

  dismissNotice() {
    this.notice = null;
  }

  // Accounts ------------------------------------------------------------------

  /** An account was added by the setup form: read the list and watch its inbox. */
  async accountAdded(account: MailAccountV1) {
    await this.loadAccounts();
    void callMail(() => commands.mailWatch(account.id, true), READ_TIMEOUT, this.lifetime.signal);
  }

  async updateAccount(update: MailAccountUpdateV1) {
    const outcome = await callMail(
      () => commands.mailUpdateAccount(update),
      WRITE_TIMEOUT,
      this.lifetime.signal,
    );
    if (outcome.ok === "aborted") return false;
    if (!outcome.ok) {
      this.notice = { action: "account", error: outcome.error };
      return false;
    }
    this.accounts = this.accounts.map((account) =>
      account.id === outcome.value.id ? outcome.value : account,
    );
    return true;
  }

  /**
   * Moves an account one place up (`-1`) or down (`1`) in the list. The
   * order is saved natively first; the list follows the answer.
   */
  async moveAccount(id: string, step: -1 | 1) {
    const ids = this.accounts.map((account) => account.id);
    const from = ids.indexOf(id);
    const to = from + step;
    if (from < 0 || to < 0 || to >= ids.length) return false;
    const order = ids.toSpliced(from, 1).toSpliced(to, 0, id);
    const outcome = await callMail(
      () => commands.mailSetAccountOrder(order),
      WRITE_TIMEOUT,
      this.lifetime.signal,
    );
    if (outcome.ok === "aborted") return false;
    if (!outcome.ok) {
      this.notice = { action: "order", error: outcome.error };
      return false;
    }
    this.accounts = order.flatMap((key) => {
      const account = this.accounts.find((candidate) => candidate.id === key);
      return account ? [account] : [];
    });
    return true;
  }

  /** Removes the account and its cache; the list follows the answer. */
  async removeAccount(id: string) {
    const outcome = await callMail(
      () => commands.mailRemoveAccount(id),
      WRITE_TIMEOUT,
      this.lifetime.signal,
    );
    if (outcome.ok === "aborted") return false;
    if (!outcome.ok) {
      this.notice = { action: "account", error: outcome.error };
      return false;
    }
    await this.loadAccounts();
    return true;
  }

  // Compose -------------------------------------------------------------------

  /** Starts a message; a reply or forward reads the open message. */
  startCompose(mode: ComposeMode): boolean {
    const original = mode === "new" ? undefined : (this.open?.body ?? undefined);
    if (mode !== "new" && !original) return false;
    const accountId =
      mode === "new"
        ? this.selection.account !== ALL_INBOXES
          ? this.selection.account
          : (this.accounts[0]?.id ?? "")
        : this.open!.row.account;
    const account = this.account(accountId) ?? this.accounts[0];
    if (!account) return false;
    this.compose?.dispose();
    this.compose = new ComposeSession(composeFor(mode, account, original), () => this.accounts);
    return true;
  }

  /**
   * Starts a new message from a parsed `mailto:` link. Only the fields the
   * link may set are used; the signature of the account the message is from
   * goes below the body as in any new message.
   */
  startComposeFromLink(link: MailtoV1): boolean {
    const accountId =
      this.selection.account !== ALL_INBOXES
        ? this.selection.account
        : (this.accounts[0]?.id ?? "");
    const account = this.account(accountId) ?? this.accounts[0];
    if (!account) return false;
    const blank = composeFor("new", account);
    const text = link.body ? withSignature(`${link.body}\n`, account.signature) : blank.text;
    this.compose?.dispose();
    this.open = null;
    this.compose = new ComposeSession(
      {
        ...blank,
        to: link.to.join(", "),
        cc: link.cc.join(", "),
        bcc: link.bcc.join(", "),
        subject: link.subject,
        text,
      },
      () => this.accounts,
    );
    return true;
  }

  /** A `mailto:` link inside a message: Rust parses it, then the message is started. */
  async composeFromLink(url: string): Promise<boolean> {
    const outcome = await callMail(
      () => commands.mailParseMailto(url),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (outcome.ok === "aborted") return false;
    if (!outcome.ok) {
      this.notice = { action: "open", error: outcome.error };
      return false;
    }
    return this.startComposeFromLink(outcome.value);
  }

  closeCompose() {
    this.compose?.dispose();
    this.compose = null;
  }
}
