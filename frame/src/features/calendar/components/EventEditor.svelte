<!--
  Create or change an event. The form holds a local draft; saving is a native
  call the page makes and settles, so a failure stays here with its reason and
  nothing is shown as saved until the backend says so. Occurrence-only editing
  would add a scope choice at the top of this form.
-->
<script lang="ts">
  import { Cancel01Icon } from "@hugeicons/core-free-icons";
  import * as m from "$shared/i18n/messages";
  import Button from "$shared/ui/Button";
  import Field from "$shared/ui/Field";
  import IconButton from "$shared/ui/IconButton";
  import Select from "$shared/ui/Select";
  import Switch from "$shared/ui/Switch";
  import TextArea from "$shared/ui/TextArea";
  import { RECURRENCES, formToDraft, type EventForm, type Recurrence } from "$domain/calendar";
  import type { CalendarFault } from "$domain/calendar";
  import type { CalendarEventDraftV1 } from "$shared/ipc/bindings";
  import { faultText, reminderLabel } from "../lib/format";

  let {
    initial,
    editing,
    calendars,
    pending,
    fault,
    onsave,
    oncancel,
  }: {
    initial: EventForm;
    /** An existing event is being changed, not a new one made. */
    editing: boolean;
    calendars: readonly { id: string; name: string }[];
    pending: boolean;
    fault: CalendarFault | null;
    onsave: (calendar: string, draft: CalendarEventDraftV1) => void;
    oncancel: () => void;
  } = $props();

  // The draft is the person's to change; `initial` only seeds it.
  // svelte-ignore state_referenced_locally
  let form = $state<EventForm>({ ...initial });
  let invalid = $state<"range" | "reminder" | null>(null);

  const recurrenceLabel: Record<Recurrence, () => string> = {
    none: m.calendar_repeat_none,
    daily: m.calendar_repeat_daily,
    weekly: m.calendar_repeat_weekly,
    monthly: m.calendar_repeat_monthly,
    yearly: m.calendar_repeat_yearly,
  };
  const repeatOptions = $derived(
    RECURRENCES.map((value) => ({ value, label: recurrenceLabel[value]() })),
  );
  const reminderOptions = $derived([
    { value: "", label: m.calendar_reminder_none() },
    ...[0, 5, 10, 15, 30, 60, 1440].map((minutes) => ({
      value: String(minutes),
      label: reminderLabel(minutes),
    })),
  ]);

  function submit(event: SubmitEvent) {
    event.preventDefault();
    if (pending) return;
    const result = formToDraft(form);
    if (!result.ok) {
      invalid = result.error;
      return;
    }
    invalid = null;
    onsave(form.calendarId, result.draft);
  }
</script>

<form
  class="editor"
  onsubmit={submit}
  aria-label={editing ? m.calendar_edit_title() : m.calendar_new_title()}
>
  <header>
    <h2>{editing ? m.calendar_edit_title() : m.calendar_new_title()}</h2>
    <IconButton
      icon={Cancel01Icon}
      label={m.calendar_cancel()}
      size={14}
      buttonSize={28}
      onclick={oncancel}
    />
  </header>

  <Field
    label={m.calendar_field_title()}
    bind:value={form.summary}
    autocomplete="off"
    maxlength={1000}
  />

  {#if !editing && calendars.length > 1}
    <Select
      label={m.calendar_field_calendar()}
      options={calendars.map((entry) => ({ value: entry.id, label: entry.name }))}
      bind:value={form.calendarId}
    />
  {/if}

  <Switch
    label={m.calendar_all_day()}
    checked={form.allDay}
    onchange={(checked) => (form.allDay = checked)}
  />

  <div class="pair">
    <Field label={m.calendar_field_start_date()} type="date" bind:value={form.startDate} required />
    {#if !form.allDay}
      <Field
        label={m.calendar_field_start_time()}
        type="time"
        bind:value={form.startTime}
        required
      />
    {/if}
  </div>
  <div class="pair">
    <Field
      label={m.calendar_field_end_date()}
      type="date"
      bind:value={form.endDate}
      required
      error={invalid === "range" ? m.calendar_error_range() : undefined}
    />
    {#if !form.allDay}
      <Field label={m.calendar_field_end_time()} type="time" bind:value={form.endTime} required />
    {/if}
  </div>

  <Field label={m.calendar_field_location()} bind:value={form.location} autocomplete="off" />
  <TextArea label={m.calendar_field_notes()} bind:value={form.description} />

  <Select label={m.calendar_field_repeat()} options={repeatOptions} bind:value={form.recurrence} />
  {#if form.recurrence !== "none"}
    <p class="hint">{m.calendar_apply_all()}</p>
  {/if}
  <Select
    label={m.calendar_field_reminder()}
    options={reminderOptions}
    bind:value={form.reminder}
  />

  {#if fault}<p class="error" role="alert">{faultText(fault)}</p>{/if}

  <div class="actions">
    <Button type="submit" variant="primary" {pending}>
      {pending ? m.calendar_saving() : m.calendar_save()}
    </Button>
    <Button disabled={pending} onclick={oncancel}>{m.calendar_cancel()}</Button>
  </div>
</form>

<style>
  .editor {
    display: grid;
    align-content: start;
    gap: 12px;
    padding: 16px;
    color: var(--color-text);
  }

  header {
    display: flex;
    align-items: center;
  }

  h2 {
    flex: 1;
    margin: 0;
    font-size: var(--text-title);
    font-weight: 600;
  }

  .pair {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
    gap: 8px;
  }

  .hint {
    margin: 0;
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  .error {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--text-label);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
</style>
