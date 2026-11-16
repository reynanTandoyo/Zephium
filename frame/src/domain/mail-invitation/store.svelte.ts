import * as m from "$shared/i18n/messages";
import { commands } from "$shared/ipc/bindings";
import type {
  CalendarInvitationActionV1,
  CalendarInvitationV1,
  CalendarItipReplyV1,
  MailAddressV1,
  MailDraftV1,
  Response,
} from "$shared/ipc/bindings";
import {
  callCalendar,
  callMail,
  READ_TIMEOUT,
  SLOW_TIMEOUT,
  WRITE_TIMEOUT,
  type CalendarFailure,
  type MailFailure,
} from "$domain/mail";
import {
  canApply,
  canRespond,
  checkSender,
  oneLine,
  type SenderCheck,
  type WritableCalendar,
} from "./checks";

type Part = "loading" | "ready" | "failed";
type Send = "idle" | "sending" | "sent" | "failed";

/** What is known about the message that carries the invitation. */
export type InvitationSource = {
  account: string;
  folder: string;
  uid: number;
  part: number;
  /** The message's From addresses, as the server gave them. */
  from: readonly MailAddressV1[];
  /** The message's Message-ID, for threading the answer. */
  messageId: string | null;
  /** The account's own addresses (the account address; it has no aliases). */
  own: readonly string[];
  /** The account's display name, for the answer's text. */
  ownName: string;
};

const sameAddress = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/** The words of the plain-text part of an answer, in the viewer's language. */
export function replyWords(response: Response, title: string, name: string) {
  const clean = oneLine(title, 200);
  const who = oneLine(name, 100);
  switch (response) {
    case "accept":
      return {
        subject: m.mail_invitation_reply_subject_accept({ title: clean }),
        text: m.mail_invitation_reply_text_accept({ name: who, title: clean }),
      };
    case "tentative":
      return {
        subject: m.mail_invitation_reply_subject_tentative({ title: clean }),
        text: m.mail_invitation_reply_text_tentative({ name: who, title: clean }),
      };
    default:
      return {
        subject: m.mail_invitation_reply_subject_decline({ title: clean }),
        text: m.mail_invitation_reply_text_decline({ name: who, title: clean }),
      };
  }
}

/**
 * One calendar invitation in an open message. The calendar side parses it
 * and applies answers; this side only asks, shows what came back and sends
 * the reply. Nothing is sent or changed until a method here is called, and
 * a request that cannot be tied to its sender is never answered or applied.
 */
export class MailInvitation {
  state = $state<Part>("loading");
  invitation = $state.raw<CalendarInvitationV1 | null>(null);
  calendars = $state.raw<readonly WritableCalendar[]>([]);
  /** Google or Microsoft calendars exist, which cannot store invited events yet. */
  providerCalendars = $state(false);
  chosen = $state("");
  /** A calendar request is in flight. */
  busy = $state(false);
  answered = $state<Response | null>(null);
  send = $state<Send>("idle");
  sendFailure = $state<MailFailure | null>(null);
  applied = $state<CalendarInvitationActionV1 | null>(null);
  calendarFailure = $state<CalendarFailure | null>(null);
  private reply: { draft: MailDraftV1; response: Response; to: string } | null = null;
  private lifetime = new AbortController();
  private generation = 0;

  constructor(private readonly source: InvitationSource) {}

  /**
   * The sender check, for display only: the backend runs the same check on the
   * From addresses given to `calendarInvitationParse` and refuses the other
   * calls when it fails. Null until the invitation is read.
   */
  get check(): SenderCheck | null {
    return this.invitation ? checkSender(this.invitation, this.source.from) : null;
  }
  get mayRespond() {
    return (
      this.invitation !== null && this.check !== null && canRespond(this.invitation, this.check)
    );
  }
  get mayApply() {
    return this.invitation !== null && this.check !== null && canApply(this.invitation, this.check);
  }
  /** Accept and Tentative need a calendar to store the event in. */
  get canStore() {
    return this.chosen !== "" && this.calendars.some((calendar) => calendar.id === this.chosen);
  }
  get sentTo() {
    return this.reply?.to ?? "";
  }

  async load() {
    const generation = this.generation;
    const { account, folder, uid, part, own } = this.source;
    const bytes = await callMail(
      () => commands.mailInvitationBytes(account, folder, uid, part),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || bytes.ok === "aborted") return;
    if (!bytes.ok) return this.fail();
    const parsed = await callCalendar(
      () => commands.calendarInvitationParse(
          bytes.value,
          [...own],
          this.source.from.map((address) => address.email),
        ),
      READ_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || parsed.ok === "aborted") return;
    if (!parsed.ok) return this.fail();
    this.invitation = parsed.value;
    await this.loadCalendars(parsed.value);
    if (generation !== this.generation) return;
    this.state = "ready";
  }

  private fail() {
    this.state = "failed";
  }

  private async loadCalendars(invitation: CalendarInvitationV1) {
    const [accounts, calendars] = await Promise.all([
      callCalendar(() => commands.calendarAccounts(), READ_TIMEOUT, this.lifetime.signal),
      callCalendar(() => commands.calendarCalendars(), READ_TIMEOUT, this.lifetime.signal),
    ]);
    if (accounts.ok !== true || calendars.ok !== true) return;
    const kinds = Object.fromEntries(accounts.value.map((account) => [account.id, account.kind]));
    this.providerCalendars = accounts.value.some(
      (account) => account.kind === "google" || account.kind === "microsoft",
    );
    const writable = calendars.value
      .filter((calendar) => !calendar.read_only && kinds[calendar.account_id] === "caldav")
      .map((calendar) => ({ id: calendar.id, name: calendar.name, color: calendar.color }));
    this.calendars = writable;
    const suggested = invitation.suggested_calendar;
    this.chosen =
      writable.find((calendar) => calendar.id === suggested)?.id ?? writable[0]?.id ?? "";
  }

  choose(id: string) {
    if (this.calendars.some((calendar) => calendar.id === id)) this.chosen = id;
  }

  /**
   * Answers a request: the calendar is updated first, then the reply goes by
   * mail from the same account. Declining needs no calendar.
   */
  async respond(response: Response): Promise<boolean> {
    const invitation = this.invitation;
    if (!invitation || this.busy || !this.mayRespond || this.send === "sending") return false;
    if (response !== "decline" && !this.canStore) return false;
    const organizer = invitation.organizer;
    if (!organizer) return false;
    this.busy = true;
    this.calendarFailure = null;
    const generation = this.generation;
    const outcome = await callCalendar(
      () => commands.calendarInvitationRespond(invitation.id, response, this.chosen || null),
      WRITE_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || outcome.ok === "aborted") return false;
    this.busy = false;
    if (!outcome.ok) {
      this.calendarFailure = outcome.error;
      return false;
    }
    this.answered = response;
    return this.queueReply(outcome.value, response, organizer);
  }

  private queueReply(
    itip: CalendarItipReplyV1,
    response: Response,
    organizer: { email: string; name: string | null },
  ) {
    // The answer goes to the organizer the message named, and nowhere else.
    if (!sameAddress(itip.to, organizer.email)) {
      this.send = "failed";
      this.sendFailure = "invalid_input";
      return false;
    }
    const words = replyWords(response, this.invitation?.summary ?? "", this.source.ownName);
    const messageId = this.source.messageId;
    this.reply = {
      response,
      to: itip.to,
      draft: {
        to: [{ name: organizer.name, email: itip.to }],
        cc: [],
        bcc: [],
        subject: words.subject,
        text: words.text,
        html: null,
        attachments: [],
        in_reply_to: messageId,
        references: messageId ? [messageId] : [],
        calendar: { method: "REPLY", ics: itip.ics },
      },
    };
    return this.sendReply();
  }

  /** Sends (or sends again) the answer queued by `respond`. */
  async sendReply(): Promise<boolean> {
    const reply = this.reply;
    if (!reply || this.send === "sending" || this.send === "sent") return false;
    this.send = "sending";
    this.sendFailure = null;
    const generation = this.generation;
    const outcome = await callMail(
      () => commands.mailSend(this.source.account, reply.draft),
      SLOW_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || outcome.ok === "aborted") return false;
    if (!outcome.ok) {
      this.send = "failed";
      this.sendFailure = outcome.error;
      return false;
    }
    this.send = "sent";
    return true;
  }

  /** Applies a verified cancellation, or an attendee's reply, to the calendar. */
  async apply(): Promise<boolean> {
    const invitation = this.invitation;
    if (!invitation || this.busy || !this.mayApply) return false;
    this.busy = true;
    this.calendarFailure = null;
    const generation = this.generation;
    const outcome = await callCalendar(
      () =>
        invitation.method === "cancel"
          ? commands.calendarInvitationApplyCancel(invitation.id)
          : commands.calendarInvitationApplyReply(invitation.id),
      WRITE_TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation || outcome.ok === "aborted") return false;
    this.busy = false;
    if (!outcome.ok) {
      this.calendarFailure = outcome.error;
      return false;
    }
    this.applied = outcome.value;
    return true;
  }

  dispose() {
    this.generation++;
    this.lifetime.abort();
    this.lifetime = new AbortController();
  }
}
