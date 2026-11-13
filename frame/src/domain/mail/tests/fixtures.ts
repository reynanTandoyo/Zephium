import type { MailAccountV1, MailFolderV1, MailMessageV1 } from "$shared/ipc/bindings";

export function message(uid: number, over: Partial<MailMessageV1> = {}): MailMessageV1 {
  return {
    uid,
    message_id: `<m${uid}@x>`,
    subject: `Subject ${uid}`,
    from: [{ name: "A", email: "a@x" }],
    to: [],
    cc: [],
    date: 1000 + uid,
    flags: { seen: true, flagged: false, answered: false, draft: false, deleted: false },
    size: 10,
    has_attachments: false,
    snippet: "",
    in_reply_to: null,
    references: [],
    ...over,
  };
}


export const account = (id = "a1", over: Partial<MailAccountV1> = {}): MailAccountV1 => ({
  id,
  display_name: `Account ${id}`,
  email: `${id}@example.com`,
  imap: { host: "imap.example.com", port: 993, security: "tls" },
  smtp: { host: "smtp.example.com", port: 465, security: "tls" },
  username: `${id}@example.com`,
  colour: "#3366cc",
  signature: "",
  save_sent_copy: true,
  ...over,
});

export const folder = (
  path: string,
  role: MailFolderV1["role"] = "other",
  unread = 0,
): MailFolderV1 => ({
  path,
  name: path,
  delimiter: "/",
  role,
  unread,
  total: unread,
  selectable: true,
});
