import { expect, test } from "vitest";
import type { MailMessageV1 } from "$shared/ipc/bindings";
import { message } from "./fixtures";
import { baseSubject, buildThreads, rowKey, type MailRow } from "../thread";

const row = (uid: number, over: Partial<MailMessageV1> = {}, account = "a1", folder = "INBOX"): MailRow => ({
  key: rowKey(account, folder, uid),
  account,
  folder,
  message: message(uid, over),
});

test("replies join their parent through In-Reply-To and References", () => {
  const rows = [
    row(1),
    row(2, { in_reply_to: "<m1@x>", references: ["<m1@x>"], subject: "Re: Subject 1" }),
    row(3, { references: ["<m1@x>", "<m2@x>"], in_reply_to: "<m2@x>", subject: "RE: Re: Subject 1" }),
    row(4),
  ];
  const threads = buildThreads(rows);
  expect(threads.map((t) => t.rows.map((r) => r.message.uid))).toEqual([[4], [1, 2, 3]]);
  expect(threads[1]!.subject).toBe("Subject 1");
  expect(threads[1]!.latest.message.uid).toBe(3);
});

test("a missing middle message still joins the ends through References", () => {
  const threads = buildThreads([
    row(1),
    row(3, { references: ["<m1@x>", "<gone@x>"], in_reply_to: "<gone@x>" }),
  ]);
  expect(threads).toHaveLength(1);
});

test("threads never cross accounts and unrelated equal subjects stay apart", () => {
  const threads = buildThreads([
    row(1, { subject: "Hello", message_id: "<same@x>" }, "a1"),
    row(2, { subject: "Hello", message_id: "<same@x>" }, "a2"),
    row(3, { subject: "Hello" }),
  ]);
  expect(threads).toHaveLength(3);
});

test("a copy of one message in two folders is one entry, and cycles terminate", () => {
  const copy = buildThreads([row(1, {}, "a1", "INBOX"), row(1, {}, "a1", "Sent")]);
  expect(copy).toHaveLength(1);
  const loop = buildThreads([
    row(1, { in_reply_to: "<m2@x>" }),
    row(2, { in_reply_to: "<m1@x>" }),
    row(3, { in_reply_to: "<m3@x>" }),
  ]);
  expect(loop.map((t) => t.rows.length).sort()).toEqual([1, 2]);
});

test("a thread counts unread, flags and attachments across its messages", () => {
  const [thread] = buildThreads([
    row(1, { flags: { seen: false, flagged: false, answered: false, draft: false, deleted: false } }),
    row(2, {
      in_reply_to: "<m1@x>",
      has_attachments: true,
      flags: { seen: true, flagged: true, answered: false, draft: false, deleted: false },
    }),
  ]);
  expect(thread).toMatchObject({ unread: 1, flagged: true, attachments: true });
});

test("threads sort newest first and ignore case and brackets in ids", () => {
  const threads = buildThreads([
    row(1, { message_id: "<AbC@x>" }),
    row(9, { date: 5 }),
    row(2, { in_reply_to: "abc@X" }),
  ]);
  expect(threads.map((t) => t.latest.message.uid)).toEqual([2, 9]);
});

test("reply prefixes are removed in several languages", () => {
  expect(baseSubject("Re: AW: Fwd: Hello")).toBe("Hello");
  expect(baseSubject("WG: RE[2]: x")).toBe("x");
  expect(baseSubject("Reply to all")).toBe("Reply to all");
});
