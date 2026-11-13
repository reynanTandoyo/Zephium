<!--
  What one event is: when, where, who, and the actions its calendar allows.
  Read-only calendars and recurring series show no edit control; the reason is
  said instead. Deleting asks first, and says when it deletes a whole series.
-->
<script lang="ts">
  import { Cancel01Icon } from "@hugeicons/core-free-icons";
  import * as m from "$shared/i18n/messages";
  import Button from "$shared/ui/Button";
  import IconButton from "$shared/ui/IconButton";
  import { editPolicy, meetingLink, type PlacedEvent } from "$domain/calendar";
  import type { CalendarFault } from "$domain/calendar";
  import { eventWhen, faultText, reminderLabel, safeColor } from "../lib/format";

  let {
    event,
    calendarName,
    pending,
    fault,
    onedit,
    ondelete,
    onjoin,
    onclose,
  }: {
    event: PlacedEvent;
    calendarName: string;
    /** A native call for this event is in flight. */
    pending: boolean;
    fault: CalendarFault | null;
    onedit: () => void;
    ondelete: () => void;
    onjoin: (url: string) => void;
    onclose: () => void;
  } = $props();

  const occurrence = $derived(event.occurrence);
  const policy = $derived(editPolicy(occurrence));
  const meeting = $derived(meetingLink(occurrence));
  let confirming = $state(false);
  // Another event was picked: the question was about the last one.
  $effect(() => {
    void event.id;
    confirming = false;
  });
</script>

<section class="details" aria-label={occurrence.summary || m.calendar_untitled()}>
  <header>
    <span class="swatch" style:--event-color={safeColor(occurrence.color)} aria-hidden="true"
    ></span>
    <h2>{occurrence.summary || m.calendar_untitled()}</h2>
    <IconButton
      icon={Cancel01Icon}
      label={m.calendar_close()}
      size={14}
      buttonSize={28}
      onclick={onclose}
    />
  </header>

  <p class="when">{eventWhen(event)}</p>
  <p class="calendar">{calendarName}{occurrence.recurring ? ` · ${m.calendar_repeats()}` : ""}</p>

  {#if meeting}
    <Button variant="primary" onclick={() => onjoin(meeting.url)}>{m.calendar_join()}</Button>
  {/if}

  <dl>
    {#if occurrence.location}
      <dt>{m.calendar_field_location()}</dt>
      <dd>{occurrence.location}</dd>
    {/if}
    {#if occurrence.description}
      <dt>{m.calendar_field_notes()}</dt>
      <dd class="notes">{occurrence.description}</dd>
    {/if}
    {#if occurrence.reminders.length > 0}
      <dt>{m.calendar_field_reminder()}</dt>
      <dd>{occurrence.reminders.map(reminderLabel).join(", ")}</dd>
    {/if}
    {#if occurrence.organizer}
      <dt>{m.calendar_organizer()}</dt>
      <dd>{occurrence.organizer.name || occurrence.organizer.email}</dd>
    {/if}
    {#if occurrence.attendees.length > 0}
      <dt>{m.calendar_attendees()}</dt>
      <dd>
        <ul>
          {#each occurrence.attendees as person (person.email)}
            <li>{person.name || person.email}</li>
          {/each}
        </ul>
      </dd>
    {/if}
  </dl>

  {#if policy === "read-only"}
    <p class="note">{m.calendar_read_only_note()}</p>
  {/if}

  {#if fault}<p class="error" role="alert">{faultText(fault)}</p>{/if}

  {#if policy === "edit"}
    {#if confirming}
      <p class="note" role="alert">{m.calendar_delete_confirm()}</p>
      <div class="actions">
        <Button variant="danger" {pending} onclick={ondelete}>{m.calendar_delete()}</Button>
        <Button disabled={pending} onclick={() => (confirming = false)}
          >{m.calendar_cancel()}</Button
        >
      </div>
    {:else}
      <div class="actions">
        <Button onclick={onedit}>{m.calendar_edit()}</Button>
        <!-- A repeating event asks which events instead of confirming here. -->
        <Button
          variant="ghost"
          onclick={() => (occurrence.recurring ? ondelete() : (confirming = true))}
          >{m.calendar_delete()}</Button
        >
      </div>
    {/if}
  {/if}
</section>

<style>
  .details {
    display: grid;
    align-content: start;
    gap: 10px;
    padding: 16px;
    color: var(--color-text);
  }

  header {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  h2 {
    flex: 1;
    margin: 0;
    font-size: var(--text-title);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .swatch {
    flex: none;
    inline-size: 10px;
    block-size: 10px;
    border-radius: var(--radius-capsule);
    background: var(--event-color);
  }

  p {
    margin: 0;
  }

  .when {
    font-size: var(--text-body);
  }

  .calendar,
  .note {
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  dl {
    display: grid;
    gap: 2px 12px;
    margin: 0;
  }

  dt {
    margin-block-start: 8px;
    color: var(--color-muted);
    font-size: var(--text-caption);
  }

  dd {
    margin: 0;
    font-size: var(--text-body);
    overflow-wrap: anywhere;
  }

  .notes {
    white-space: pre-wrap;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .error {
    color: var(--color-danger);
    font-size: var(--text-label);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
</style>
