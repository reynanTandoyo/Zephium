import { commands } from "$shared/ipc/bindings";
import type { MailAccountV1, MailOutgoingAttachmentV1 } from "$shared/ipc/bindings";
import { callMail, SLOW_TIMEOUT, WRITE_TIMEOUT, type MailFailure } from "./call";
import {
  buildDraft,
  bytesToBase64,
  MAX_ATTACHMENT_BYTES,
  MAX_TOTAL_ATTACHMENT_BYTES,
  safeFilename,
  swapSignature,
  type ComposeFields,
  type DraftCheck,
} from "./compose";

/**
 * Where a message stands. `sent` is set only when the server accepted it; a
 * timeout leaves `failed` with an unknown outcome, never `sent`.
 */
export type ComposeState = "editing" | "saving" | "sending" | "sent" | "failed";

export type ComposeAttachment = MailOutgoingAttachmentV1 & { size: number };

export type ComposeProblem = Extract<DraftCheck, { ok: false }>["problem"] | "attachment_too_large";

/** One message being written. Rust stores drafts; this holds the text meanwhile. */
export class ComposeSession {
  account = $state("");
  to = $state("");
  cc = $state("");
  bcc = $state("");
  subject = $state("");
  text = $state("");
  attachments = $state.raw<readonly ComposeAttachment[]>([]);
  state = $state<ComposeState>("editing");
  failure = $state<MailFailure | null>(null);
  problem = $state<ComposeProblem | null>(null);
  invalid = $state<readonly string[]>([]);
  /** The draft was stored on the server's Drafts folder since the last edit. */
  saved = $state(false);
  /** The last send ended without an answer, so it may have gone out. */
  unknownOutcome = $state(false);
  /** The Sent copy was stored along with the send. */
  savedCopy = $state(false);
  private readonly inReplyTo: string | null;
  private readonly references: string[];
  private signature: string;
  private lifetime = new AbortController();
  private generation = 0;

  constructor(
    fields: ComposeFields,
    private readonly accounts: () => readonly MailAccountV1[],
  ) {
    this.account = fields.account;
    this.to = fields.to;
    this.cc = fields.cc;
    this.bcc = fields.bcc;
    this.subject = fields.subject;
    this.text = fields.text;
    this.inReplyTo = fields.in_reply_to;
    this.references = fields.references;
    this.signature = this.accounts().find((a) => a.id === fields.account)?.signature ?? "";
  }

  get busy() {
    return this.state === "saving" || this.state === "sending";
  }
  get dirty() {
    return !this.saved && this.state !== "sent" && Boolean(this.to || this.subject || this.text.trim());
  }

  /** Any edit means the stored draft is out of date. */
  edited() {
    this.saved = false;
    this.problem = null;
    if (this.state === "failed") this.state = "editing";
  }

  /** The From picker changed: the signature follows the account. */
  setAccount(id: string) {
    if (id === this.account) return;
    const next = this.accounts().find((a) => a.id === id);
    if (!next) return;
    this.text = swapSignature(this.text, this.signature, next.signature);
    this.signature = next.signature;
    this.account = id;
    this.edited();
  }

  private fields(): ComposeFields {
    return {
      account: this.account,
      to: this.to,
      cc: this.cc,
      bcc: this.bcc,
      subject: this.subject,
      text: this.text,
      in_reply_to: this.inReplyTo,
      references: this.references,
    };
  }

  private check(requireRecipient: boolean): DraftCheck {
    const attachments = this.attachments;
    return buildDraft(
      this.fields(),
      attachments.map(({ filename, content_type, data_base64 }) => ({
        filename,
        content_type,
        data_base64,
      })),
      attachments.map((a) => a.size),
      requireRecipient,
    );
  }

  /** Reads a chosen file into the message, within the size caps. */
  async attach(file: File): Promise<boolean> {
    const used = this.attachments.reduce((sum, a) => sum + a.size, 0);
    if (file.size > MAX_ATTACHMENT_BYTES || used + file.size > MAX_TOTAL_ATTACHMENT_BYTES) {
      this.problem = "attachment_too_large";
      return false;
    }
    const bytes = new Uint8Array(await file.arrayBuffer());
    this.attachments = [
      ...this.attachments,
      {
        filename: safeFilename(file.name),
        content_type: file.type || "application/octet-stream",
        data_base64: bytesToBase64(bytes),
        size: bytes.length,
      },
    ];
    this.edited();
    return true;
  }

  detach(index: number) {
    this.attachments = this.attachments.filter((_, at) => at !== index);
    this.edited();
  }

  /** Stores the draft in the Drafts folder. */
  async saveDraft(): Promise<boolean> {
    if (this.busy) return false;
    const checked = this.check(false);
    if (!checked.ok) return this.refuse(checked);
    this.state = "saving";
    this.failure = null;
    this.problem = null;
    const generation = this.generation;
    const outcome = await callMail(
      () => commands.mailSaveDraft(this.account, checked.draft),
      WRITE_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || outcome.ok === "aborted") return false;
    if (!outcome.ok) {
      this.state = "failed";
      this.failure = outcome.error;
      return false;
    }
    this.state = "editing";
    this.saved = true;
    return true;
  }

  /** Sends the message. `sent` follows the server's answer, never before. */
  async send(): Promise<boolean> {
    if (this.busy || this.state === "sent") return false;
    const checked = this.check(true);
    if (!checked.ok) return this.refuse(checked);
    this.state = "sending";
    this.failure = null;
    this.problem = null;
    this.unknownOutcome = false;
    const generation = this.generation;
    const outcome = await callMail(
      () => commands.mailSend(this.account, checked.draft),
      SLOW_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || outcome.ok === "aborted") return false;
    if (!outcome.ok) {
      this.state = "failed";
      this.failure = outcome.error;
      this.unknownOutcome = outcome.error === "timeout";
      return false;
    }
    this.state = "sent";
    this.savedCopy = outcome.value.saved_copy;
    return true;
  }

  private refuse(checked: Extract<DraftCheck, { ok: false }>) {
    this.problem = checked.problem;
    this.invalid = checked.invalid;
    this.state = "editing";
    return false;
  }

  dispose() {
    this.generation++;
    this.lifetime.abort();
    this.lifetime = new AbortController();
    this.attachments = [];
  }
}
