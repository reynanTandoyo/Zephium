<!--
  The calendars, grouped by account, each with its colour and a visibility
  checkbox. The checkbox follows what native says: a refused change snaps back.
-->
<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import { calendar } from "$domain/calendar";
  import type { CalendarFault } from "$domain/calendar";
  import { faultText, safeColor } from "../lib/format";

  let fault = $state<CalendarFault | null>(null);

  const groups = $derived(
    calendar
      .accounts()
      .map((account) => ({
        account,
        calendars: calendar.calendars().filter((entry) => entry.account_id === account.id),
      }))
      .filter((group) => group.calendars.length > 0),
  );

  async function toggle(event: Event, id: string, visible: boolean) {
    const box = event.currentTarget as HTMLInputElement;
    fault = null;
    const result = await calendar.setVisible(id, visible);
    if (!result.ok) {
      fault = result.error;
      box.checked = !visible;
    }
  }
</script>

<section class="calendars" aria-label={m.calendar_calendars()}>
  <h2>{m.calendar_calendars()}</h2>
  {#each groups as group (group.account.id)}
    <div class="group">
      <h3>{group.account.name}</h3>
      <ul>
        {#each group.calendars as entry (entry.id)}
          <li style:--calendar-color={safeColor(entry.color)}>
            <label>
              <input
                type="checkbox"
                checked={entry.visible}
                disabled={calendar.busy() === "visibility"}
                onchange={(event) => void toggle(event, entry.id, event.currentTarget.checked)}
              />
              <span class="name">{entry.name}</span>
              {#if entry.read_only}<span class="lock">{m.calendar_read_only()}</span>{/if}
            </label>
          </li>
        {/each}
      </ul>
    </div>
  {/each}
  {#if fault}<p class="error" role="alert">{faultText(fault)}</p>{/if}
</section>

<style>
  .calendars {
    display: grid;
    gap: 8px;
    color: var(--color-text);
  }

  h2 {
    margin: 0;
    font-size: var(--text-label);
    font-weight: 600;
  }

  h3 {
    margin: 0 0 2px;
    overflow: hidden;
    color: var(--color-muted);
    font-size: var(--text-caption);
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  ul {
    display: grid;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  label {
    display: flex;
    align-items: center;
    gap: 8px;
    min-block-size: 28px;
    font-size: var(--text-label);
    cursor: pointer;
  }

  input {
    flex: none;
    inline-size: 16px;
    block-size: 16px;
    margin: 0;
    accent-color: var(--calendar-color);
  }

  input:focus-visible {
    outline: 2px solid var(--color-ring);
    outline-offset: 2px;
  }

  .name {
    flex: 1;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .lock {
    color: var(--color-faint);
    font-size: var(--text-caption);
  }

  .error {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--text-caption);
  }
</style>
