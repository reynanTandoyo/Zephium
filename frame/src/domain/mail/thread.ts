import type { MailMessageV1 } from "$shared/ipc/bindings";

/** One message of the list with where it lives. */
export type MailRow = {
  /** Unique across accounts and folders: `account/folder/uid`. */
  key: string;
  account: string;
  folder: string;
  message: MailMessageV1;
};

export type MailThread = {
  /** The key of the newest row; stable while the thread keeps its newest message. */
  key: string;
  /** Oldest first. */
  rows: MailRow[];
  /** The newest row, which stands for the thread in the list. */
  latest: MailRow;
  subject: string;
  unread: number;
  flagged: boolean;
  attachments: boolean;
};

export const rowKey = (account: string, folder: string, uid: number) =>
  `${account}/${folder}/${uid}`;

/** Strips reply and forward prefixes, in the common languages, for display. */
export function baseSubject(subject: string): string {
  let value = subject.trim();
  const prefix = /^(?:re|fw|fwd|aw|wg|sv|vs|r|tr|rv|enc|res)\s*(?:\[\d+\])?\s*:\s*/iu;
  while (prefix.test(value)) value = value.replace(prefix, "");
  return value.trim();
}

const idOf = (value: string | null | undefined) => {
  const trimmed = value?.trim().replace(/^<|>$/gu, "");
  return trimmed ? trimmed.toLowerCase() : null;
};

/**
 * Groups messages into threads by `References` and `In-Reply-To`. Messages
 * are joined when one names the other's Message-ID; threads never cross
 * accounts. Subjects are not used to join, so two unrelated "Hello" mails
 * stay apart. Newest thread first.
 */
export function buildThreads(rows: readonly MailRow[]): MailThread[] {
  const parent = rows.map((_, index) => index);
  const find = (index: number): number => {
    let root = index;
    while (parent[root] !== root) root = parent[root]!;
    let walk = index;
    while (parent[walk] !== root) {
      const next = parent[walk]!;
      parent[walk] = root;
      walk = next;
    }
    return root;
  };
  const union = (a: number, b: number) => {
    const rootA = find(a);
    const rootB = find(b);
    if (rootA !== rootB) parent[Math.max(rootA, rootB)] = Math.min(rootA, rootB);
  };

  const byId = new Map<string, number[]>();
  rows.forEach((row, index) => {
    const id = idOf(row.message.message_id);
    if (!id) return;
    const key = `${row.account}\n${id}`;
    const list = byId.get(key);
    if (list) list.push(index);
    else byId.set(key, [index]);
  });
  rows.forEach((row, index) => {
    const own = idOf(row.message.message_id);
    const named = [...row.message.references, row.message.in_reply_to ?? ""];
    for (const raw of named) {
      const id = idOf(raw);
      if (!id || id === own) continue;
      for (const other of byId.get(`${row.account}\n${id}`) ?? []) union(index, other);
    }
    // The same message in two folders (a copy in Sent and Inbox) is one entry.
    if (own) for (const other of byId.get(`${row.account}\n${own}`) ?? []) union(index, other);
  });

  const groups = new Map<number, MailRow[]>();
  rows.forEach((row, index) => {
    const root = find(index);
    const list = groups.get(root);
    if (list) list.push(row);
    else groups.set(root, [row]);
  });

  const date = (row: MailRow) => row.message.date ?? 0;
  const threads: MailThread[] = [];
  for (const group of groups.values()) {
    group.sort((a, b) => date(a) - date(b) || a.message.uid - b.message.uid);
    const latest = group[group.length - 1]!;
    threads.push({
      key: latest.key,
      rows: group,
      latest,
      subject: baseSubject(group[0]!.message.subject) || group[0]!.message.subject,
      unread: group.filter((row) => !row.message.flags.seen).length,
      flagged: group.some((row) => row.message.flags.flagged),
      attachments: group.some((row) => row.message.has_attachments),
    });
  }
  threads.sort((a, b) => date(b.latest) - date(a.latest) || b.latest.message.uid - a.latest.message.uid);
  return threads;
}
