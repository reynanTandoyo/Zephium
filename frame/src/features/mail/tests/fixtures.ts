import type { MailAccountV1, MailBodyV1, MailFolderV1, MailMessageV1 } from "$shared/ipc/bindings";

export const account: MailAccountV1 = {
  id: "acc-1",
  display_name: "Work",
  email: "me@example.com",
  imap: { host: "imap.example.com", port: 993, security: "tls" },
  smtp: { host: "smtp.example.com", port: 465, security: "tls" },
  username: "me@example.com",
  colour: "#3b6fd4",
  signature: "",
  save_sent_copy: true,
};

const folder = (path: string, role: MailFolderV1["role"], unread = 0): MailFolderV1 => ({
  path,
  name: path,
  delimiter: "/",
  role,
  unread,
  total: unread,
  selectable: true,
});

export const folders: MailFolderV1[] = [
  folder("INBOX", "inbox", 1),
  folder("Sent", "sent"),
  folder("Archive", "archive"),
  folder("Trash", "trash"),
];

export function message(uid: number, over: Partial<MailMessageV1> = {}): MailMessageV1 {
  return {
    uid,
    message_id: `<m${uid}@x>`,
    subject: `Subject ${uid}`,
    from: [{ name: "Ann Example", email: "ann@example.org" }],
    to: [{ name: null, email: "me@example.com" }],
    cc: [],
    date: Math.floor(Date.now() / 1000) - uid * 60,
    flags: { seen: true, flagged: false, answered: false, draft: false, deleted: false },
    size: 1200,
    has_attachments: false,
    snippet: `Snippet ${uid}`,
    in_reply_to: null,
    references: [],
    ...over,
  };
}

export function body(header: MailMessageV1, over: Partial<MailBodyV1> = {}): MailBodyV1 {
  return {
    header,
    reply_to: [],
    text: "Plain body",
    html: null,
    truncated: false,
    attachments: [],
    inline_images: [],
    invitations: [],
    ...over,
  };
}

export const ok = <T>(data: T) => ({ status: "ok" as const, data });
export const failed = (error: string) => ({ status: "error" as const, error });
export const pageOf = (messages: MailMessageV1[]) =>
  ok({ messages, next_before: null, total: messages.length, from_cache: false });
