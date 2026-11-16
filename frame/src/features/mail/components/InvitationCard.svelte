<!--
  A calendar invitation above the message it came in: what, when, where, who,
  and what your calendar already knows about it. Nothing is answered, stored
  or sent until a button here is pressed. A request whose sender is not its
  organizer (or a reply whose sender is not the attendee) is shown with a
  warning and never applied.
-->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import Calendar03Icon from "@hugeicons/core-free-icons/Calendar03Icon";
  import Cancel01Icon from "@hugeicons/core-free-icons/Cancel01Icon";
  import Clock01Icon from "@hugeicons/core-free-icons/Clock01Icon";
  import HelpCircleIcon from "@hugeicons/core-free-icons/HelpCircleIcon";
  import Tick02Icon from "@hugeicons/core-free-icons/Tick02Icon";
  import * as m from "$shared/i18n/messages";
  import { MailInvitation, timeText, type InvitationSource } from "$domain/mail-invitation";
  import Button from "$shared/ui/Button";
  import Icon from "$shared/ui/Icon";
  import Select from "$shared/ui/Select";
  import { reasonText } from "../lib/text";

  let { source }: { source: InvitationSource } = $props();

  // One card per message part; the parent keys it, so this never outlives its source.
  // svelte-ignore state_referenced_locally
  const card = new MailInvitation(source);
  onMount(() => void card.load());
  onDestroy(() => card.dispose());

  const invitation = $derived(card.invitation);
  const check = $derived(card.check);

  const kind = $derived.by(() => {
    switch (invitation?.method) {
      case "cancel":
        return m.mail_invitation_kind_cancel();
      case "reply":
        return m.mail_invitation_kind_reply();
      case "counter":
        return m.mail_invitation_kind_counter();
      default:
        return invitation?.existing?.freshness === "newer"
          ? m.mail_invitation_kind_update()
          : m.mail_invitation_kind_request();
    }
  });

  const existing = $derived.by(() => {
    const stored = invitation?.existing;
    if (!stored) return null;
    if (stored.status === "cancelled") return m.mail_invitation_existing_cancelled();
    if (stored.freshness === "newer") return m.mail_invitation_existing_newer();
    if (stored.freshness === "older") return m.mail_invitation_existing_older();
    return m.mail_invitation_existing();
  });

  const warning = $derived.by(() => {
    if (!check || check.ok) return null;
    if (check.reason === "no_organizer") return m.mail_invitation_no_organizer();
    return m.mail_invitation_untrusted({
      from: source.from.map((address) => address.email).join(", ") || "?",
      named: check.named || "?",
    });
  });

  const statusIcon = (status: string) =>
    status === "accepted"
      ? Tick02Icon
      : status === "declined"
        ? Cancel01Icon
        : status === "tentative"
          ? HelpCircleIcon
          : Clock01Icon;

  const statusText = (status: string) => {
    switch (status) {
      case "accepted":
        return m.mail_invitation_status_accepted();
      case "declined":
        return m.mail_invitation_status_declined();
      case "tentative":
        return m.mail_invitation_status_tentative();
      default:
        return m.mail_invitation_status_needs_action();
    }
  };

  const answeredText = $derived.by(() => {
    const to = card.sentTo;
    switch (card.answered) {
      case "accept":
        return m.mail_invitation_answered_accept({ to });
      case "tentative":
        return m.mail_invitation_answered_tentative({ to });
      case "decline":
        return m.mail_invitation_answered_decline({ to });
      default:
        return "";
    }
  });

  const appliedText = $derived.by(() => {
    switch (card.applied) {
      case "created":
        return m.mail_invitation_applied_created();
      case "updated":
        return m.mail_invitation_applied_updated();
      case "removed":
        return m.mail_invitation_applied_removed();
      case "cancelled":
        return m.mail_invitation_applied_cancelled();
      case "none":
        return m.mail_invitation_applied_none();
      default:
        return "";
    }
  });

  const calendarOptions = $derived(
    card.calendars.map((calendar) => ({ value: calendar.id, label: calendar.name })),
  );
  const answering = $derived(card.busy || card.send === "sending");
  const finished = $derived(card.send === "sent");
</script>

<section class="card" aria-label={m.mail_invitation_label()}>
  {#if card.state === "loading"}
    <p class="muted" role="status">{m.mail_loading()}</p>
  {:else if card.state === "failed" || !invitation}
    <p class="muted" role="status">{m.mail_invitation_read_failed()}</p>
  {:else}
    <header>
      <Icon icon={Calendar03Icon} size={18} />
      <span class="kind">{kind}</span>
    </header>
    <h3>{invitation.summary || m.mail_no_subject()}</h3>

    {#if warning}<p class="warn" role="alert">{warning}</p>{/if}

    <dl>
      <dt>{m.mail_invitation_when()}</dt>
      <dd>{timeText(invitation)}</dd>
      {#if invitation.location}
        <dt>{m.mail_invitation_where()}</dt>
        <dd>{invitation.location}</dd>
      {/if}
      {#if invitation.organizer}
        <dt>{m.mail_invitation_organizer()}</dt>
        <dd>{invitation.organizer.name || invitation.organizer.email}</dd>
      {/if}
    </dl>

    {#if invitation.attendees.length}
      <details class="people">
        <summary>{m.mail_invitation_attendees({ count: invitation.attendees.length })}</summary>
        <ul>
          {#each invitation.attendees as attendee (attendee.person.email)}
            <li>
              <Icon icon={statusIcon(attendee.person.partstat)} size={14} />
              <span class="who">{attendee.person.name || attendee.person.email}</span>
              {#if attendee.is_you}<span class="muted">({m.mail_invitation_you()})</span>{/if}
              <span class="muted">{statusText(attendee.person.partstat)}</span>
            </li>
          {/each}
        </ul>
      </details>
    {/if}

    {#if existing}<p class="existing" role="status">{existing}</p>{/if}
    {#if invitation.method === "counter"}
      <p class="muted">{m.mail_invitation_counter_note()}</p>
    {/if}

    {#if card.mayRespond && !finished}
      {#if calendarOptions.length}
        <Select
          label={m.mail_invitation_calendar()}
          options={calendarOptions}
          value={card.chosen}
          onchange={(id) => card.choose(id)}
          disabled={answering}
        />
      {:else}
        <p class="muted">
          {m.mail_invitation_no_calendar()}
          {#if card.providerCalendars}{m.mail_invitation_provider_note()}{/if}
        </p>
      {/if}
      <div class="bar" role="group" aria-label={m.mail_invitation_label()}>
        <Button
          variant="primary"
          disabled={answering || !card.canStore}
          onclick={() => void card.respond("accept")}>{m.mail_invitation_accept()}</Button
        >
        <Button
          disabled={answering || !card.canStore}
          onclick={() => void card.respond("tentative")}>{m.mail_invitation_tentative()}</Button
        >
        <Button disabled={answering} onclick={() => void card.respond("decline")}
          >{m.mail_invitation_decline()}</Button
        >
      </div>
    {/if}

    {#if card.mayApply && card.applied === null}
      <div class="bar">
        <Button variant="primary" pending={card.busy} onclick={() => void card.apply()}>
          {invitation.method === "cancel"
            ? m.mail_invitation_apply_cancel()
            : m.mail_invitation_apply_reply()}
        </Button>
      </div>
    {/if}

    <div class="status" aria-live="polite">
      {#if card.send === "sending"}<p class="muted" role="status">
          {m.mail_invitation_sending()}
        </p>{/if}
      {#if finished}<p role="status">{answeredText}</p>{/if}
      {#if card.send === "failed" && card.sendFailure}
        <p class="warn" role="alert">
          {m.mail_invitation_send_failed({ reason: reasonText(card.sendFailure) })}
        </p>
        {#if card.sentTo}
          <Button size="compact" onclick={() => void card.sendReply()}
            >{m.mail_invitation_send_again()}</Button
          >
        {/if}
      {/if}
      {#if card.calendarFailure}
        <p class="warn" role="alert">
          {m.mail_invitation_calendar_failed({ code: card.calendarFailure })}
        </p>
      {/if}
      {#if card.applied !== null}<p role="status">{appliedText}</p>{/if}
    </div>
  {/if}
</section>

<style>
  .card {
    display: grid;
    gap: 10px;
    margin: 0 0 12px;
    padding: 14px 16px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
    background: var(--color-fill);
    color: var(--color-text);
    font-size: var(--text-body);
  }

  header {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--color-muted);
    font-size: var(--text-label);
    font-weight: 500;
  }

  h3 {
    margin: 0;
    font-size: var(--text-page-title);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  p {
    margin: 0;
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 12px;
    margin: 0;
  }

  dt {
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .people summary {
    cursor: pointer;
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  .people ul {
    display: grid;
    gap: 4px;
    margin: 8px 0 0;
    padding: 0;
    list-style: none;
  }

  .people li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
  }

  .who {
    overflow-wrap: anywhere;
  }

  .existing {
    font-weight: 500;
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .status {
    display: grid;
    gap: 6px;
  }

  .muted {
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  .warn {
    color: var(--color-danger);
  }
</style>
