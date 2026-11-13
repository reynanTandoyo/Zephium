import type {
  MailAccountV1,
  MailAddressV1,
  MailBodyV1,
  MailDraftV1,
  MailOutgoingAttachmentV1,
} from "$shared/ipc/bindings";
import { formatAddress, formatAddresses, parseAddresses } from "./address";
import { baseSubject } from "./thread";

export type ComposeMode = "new" | "reply" | "reply_all" | "forward";

/** One attachment is at most this large, and a message at most the total. */
export const MAX_ATTACHMENT_BYTES = 15 * 1024 * 1024;
export const MAX_TOTAL_ATTACHMENT_BYTES = 20 * 1024 * 1024;
export const MAX_RECIPIENTS = 100;

export type ComposeFields = {
  account: string;
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  text: string;
  in_reply_to: string | null;
  references: string[];
};

export const SIGNATURE_MARK = "\n-- \n";

/** Appends the account's signature, once, below the body (above quoted text). */
export function withSignature(text: string, signature: string, quoteAt?: number): string {
  const sig = signature.trim();
  if (!sig) return text;
  const block = `${SIGNATURE_MARK}${sig}\n`;
  if (text.includes(block)) return text;
  if (quoteAt !== undefined && quoteAt >= 0) return text.slice(0, quoteAt) + block + text.slice(quoteAt);
  return text + block;
}

/** Swaps one account's signature for another's when the From picker changes. */
export function swapSignature(text: string, from: string, to: string): string {
  const old = from.trim() ? `${SIGNATURE_MARK}${from.trim()}\n` : "";
  const next = to.trim() ? `${SIGNATURE_MARK}${to.trim()}\n` : "";
  if (old && text.includes(old)) return text.replace(old, next);
  return next ? withSignature(text, to) : text;
}

const dateLine = (seconds: number | null) =>
  seconds === null ? "" : new Date(seconds * 1000).toLocaleString();

/** Quotes the original with `> ` and an attribution line, as a reply does. */
export function quoteText(body: MailBodyV1): string {
  const header = body.header;
  const who = header.from[0] ? formatAddress(header.from[0]) : "";
  const when = dateLine(header.date);
  const attribution = [when, who].filter(Boolean).join(", ");
  const original = (body.text ?? header.snippet).replace(/\r\n?/gu, "\n").trimEnd();
  const quoted = original
    .split("\n")
    .map((line) => (line ? `> ${line}` : ">"))
    .join("\n");
  return `${attribution ? `${attribution} wrote:\n` : ""}${quoted}\n`;
}

function forwardText(body: MailBodyV1): string {
  const header = body.header;
  const lines = [
    "---------- Forwarded message ----------",
    `From: ${formatAddresses(header.from)}`,
    `Date: ${dateLine(header.date)}`,
    `Subject: ${header.subject}`,
    `To: ${formatAddresses(header.to)}`,
    "",
    (body.text ?? header.snippet).replace(/\r\n?/gu, "\n").trimEnd(),
  ];
  return lines.join("\n") + "\n";
}

const prefixed = (prefix: "Re" | "Fwd", subject: string) => {
  const base = baseSubject(subject) || subject;
  return `${prefix}: ${base}`;
};

const sameAddress = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/** The fields a new message, reply, reply-all or forward starts with. */
export function composeFor(
  mode: ComposeMode,
  account: MailAccountV1,
  original?: MailBodyV1,
): ComposeFields {
  const blank: ComposeFields = {
    account: account.id,
    to: "",
    cc: "",
    bcc: "",
    subject: "",
    text: "",
    in_reply_to: null,
    references: [],
  };
  if (mode === "new" || !original) return { ...blank, text: withSignature("", account.signature) };
  const header = original.header;
  const messageId = header.message_id;
  const references = [...header.references, ...(messageId ? [messageId] : [])];
  if (mode === "forward") {
    const text = `\n${forwardText(original)}`;
    return {
      ...blank,
      subject: prefixed("Fwd", header.subject),
      text: withSignature(text, account.signature, 0),
      references,
    };
  }
  const replyTo = original.reply_to.length ? original.reply_to : header.from;
  const mine = (address: MailAddressV1) => sameAddress(address.email, account.email);
  let to = replyTo.filter((address) => !mine(address));
  if (!to.length) to = [...replyTo];
  let cc: MailAddressV1[] = [];
  if (mode === "reply_all") {
    const seen = new Set(to.map((address) => address.email.toLowerCase()));
    cc = [...header.to, ...header.cc].filter((address) => {
      const key = address.email.toLowerCase();
      if (mine(address) || seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }
  return {
    ...blank,
    to: formatAddresses(to),
    cc: formatAddresses(cc),
    subject: prefixed("Re", header.subject),
    text: withSignature(`\n\n${quoteText(original)}`, account.signature, 0),
    in_reply_to: messageId,
    references,
  };
}

export type DraftCheck =
  | { ok: true; draft: MailDraftV1 }
  | { ok: false; problem: "no_recipient" | "invalid_address" | "too_many" | "too_large"; invalid: string[] };

/** Builds the wire draft, or says why it cannot be sent. */
export function buildDraft(
  fields: ComposeFields,
  attachments: readonly MailOutgoingAttachmentV1[],
  sizes: readonly number[],
  requireRecipient: boolean,
): DraftCheck {
  const to = parseAddresses(fields.to);
  const cc = parseAddresses(fields.cc);
  const bcc = parseAddresses(fields.bcc);
  const invalid = [...to.invalid, ...cc.invalid, ...bcc.invalid];
  if (invalid.length) return { ok: false, problem: "invalid_address", invalid };
  const count = to.addresses.length + cc.addresses.length + bcc.addresses.length;
  if (count > MAX_RECIPIENTS) return { ok: false, problem: "too_many", invalid: [] };
  if (requireRecipient && count === 0) return { ok: false, problem: "no_recipient", invalid: [] };
  if (sizes.reduce((sum, size) => sum + size, 0) > MAX_TOTAL_ATTACHMENT_BYTES) {
    return { ok: false, problem: "too_large", invalid: [] };
  }
  return {
    ok: true,
    draft: {
      to: to.addresses,
      cc: cc.addresses,
      bcc: bcc.addresses,
      subject: fields.subject.trim(),
      text: fields.text,
      html: null,
      attachments: [...attachments],
      in_reply_to: fields.in_reply_to,
      references: fields.references,
    },
  };
}

/** Base64 of bytes, in chunks so a large file does not overflow the call stack. */
export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let at = 0; at < bytes.length; at += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
  }
  return btoa(binary);
}

/** A bare file name: no folders, no control characters, bounded. */
export function safeFilename(name: string): string {
  // eslint-disable-next-line no-control-regex
  const cleaned = name.replace(/[\u0000-\u001f\u007f"<>:|?*\\/]/gu, "_").trim();
  return (cleaned || "attachment").slice(0, 180);
}
