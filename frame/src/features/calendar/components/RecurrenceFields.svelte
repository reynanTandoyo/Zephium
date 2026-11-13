<!--
  The repeat rule of an event or series: how often, which weekdays, when it
  ends. A rule the form cannot express is shown as it is and kept untouched.
-->
<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import Field from "$shared/ui/Field";
  import Select from "$shared/ui/Select";
  import { WEEKDAYS, type RecurrenceForm } from "$domain/calendar";
  import { weekdayShort } from "../lib/format";

  let {
    recurrence = $bindable(),
    error,
  }: {
    recurrence: RecurrenceForm;
    error?: string;
  } = $props();

  const freqOptions = $derived([
    { value: "none", label: m.calendar_repeat_none() },
    { value: "daily", label: m.calendar_repeat_daily() },
    { value: "weekly", label: m.calendar_repeat_weekly() },
    { value: "monthly", label: m.calendar_repeat_monthly() },
    { value: "yearly", label: m.calendar_repeat_yearly() },
  ]);
  const endsOptions = $derived([
    { value: "never", label: m.calendar_ends_never() },
    { value: "count", label: m.calendar_ends_count() },
    { value: "until", label: m.calendar_ends_until() },
  ]);
  const unit = $derived(
    recurrence.freq === "daily"
      ? m.calendar_unit_days()
      : recurrence.freq === "weekly"
        ? m.calendar_unit_weeks()
        : recurrence.freq === "monthly"
          ? m.calendar_unit_months()
          : m.calendar_unit_years(),
  );
  // 2026-01-05 is a Monday: the names come from the interface language.
  const dayName = (index: number) => weekdayShort(new Date(2026, 0, 5 + index));

  function toggle(day: string, on: boolean) {
    recurrence.byDay = on
      ? WEEKDAYS.filter((entry) => entry === day || recurrence.byDay.includes(entry))
      : recurrence.byDay.filter((entry) => entry !== day);
  }
</script>

{#if recurrence.freq === "custom"}
  <div class="custom">
    <span class="label">{m.calendar_field_repeat()}</span>
    <p>{m.calendar_repeat_custom()}</p>
    {#if recurrence.rrule}<code>{recurrence.rrule}</code>{/if}
  </div>
{:else}
  <Select
    label={m.calendar_field_repeat()}
    options={freqOptions}
    value={recurrence.freq}
    onchange={(value) => (recurrence.freq = value as RecurrenceForm["freq"])}
  />
  {#if recurrence.freq !== "none"}
    <div class="every">
      <Field
        label={m.calendar_field_interval()}
        type="number"
        min="1"
        max="1000"
        inputmode="numeric"
        value={recurrence.interval}
        oninput={(event) => (recurrence.interval = event.currentTarget.value)}
      />
      <span class="unit">{unit}</span>
    </div>
    {#if recurrence.freq === "weekly"}
      <fieldset class="days">
        <legend>{m.calendar_repeat_on()}</legend>
        {#each WEEKDAYS as day, index (day)}
          <label>
            <input
              type="checkbox"
              checked={recurrence.byDay.includes(day)}
              onchange={(event) => toggle(day, event.currentTarget.checked)}
            />
            <span>{dayName(index)}</span>
          </label>
        {/each}
      </fieldset>
    {/if}
    <Select
      label={m.calendar_field_ends()}
      options={endsOptions}
      value={recurrence.ends}
      onchange={(value) => (recurrence.ends = value as RecurrenceForm["ends"])}
    />
    {#if recurrence.ends === "count"}
      <Field
        label={m.calendar_field_count()}
        type="number"
        min="1"
        max="1000"
        inputmode="numeric"
        value={recurrence.count}
        oninput={(event) => (recurrence.count = event.currentTarget.value)}
      />
    {:else if recurrence.ends === "until"}
      <Field label={m.calendar_field_until()} type="date" bind:value={recurrence.until} />
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  {/if}
{/if}

<style>
  .custom {
    display: grid;
    gap: 4px;
  }

  .label {
    color: var(--color-muted);
    font-size: var(--text-label);
    font-weight: 500;
  }

  .custom p {
    margin: 0;
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  code {
    overflow-wrap: anywhere;
    padding: 4px 8px;
    border-radius: var(--radius-inset);
    background: var(--color-fill);
    font-family: var(--font-mono);
    font-size: var(--text-caption);
  }

  .every {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }

  .every :global(.ui-field) {
    flex: 0 0 96px;
  }

  .unit {
    padding-block-end: 8px;
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  .days {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    margin: 0;
    padding: 0;
    border: 0;
  }

  legend {
    padding: 0;
    margin-block-end: 4px;
    color: var(--color-muted);
    font-size: var(--text-label);
    font-weight: 500;
  }

  .days label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-label);
  }

  .days input {
    margin: 0;
    accent-color: var(--color-accent);
  }

  .days input:focus-visible {
    outline: 2px solid var(--color-ring);
    outline-offset: 2px;
  }

  .error {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--text-label);
  }
</style>
