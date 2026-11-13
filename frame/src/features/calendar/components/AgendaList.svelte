<!--
  Agenda: the next days that hold events, one heading per day.
-->
<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import EmptyState from "$shared/ui/EmptyState";
  import { agendaDays, dayKey, sameDay, type PlacedEvent } from "$domain/calendar";
  import { dayHeading, safeColor, timeLabel } from "../lib/format";

  let {
    days,
    events,
    selected,
    onselect,
  }: {
    days: readonly Date[];
    events: readonly PlacedEvent[];
    selected: string | null;
    onselect: (event: PlacedEvent) => void;
  } = $props();

  const groups = $derived(agendaDays(events, days));
  const today = new Date();
</script>

{#if groups.length === 0}
  <div class="empty">
    <EmptyState title={m.calendar_agenda_empty_title()} description={m.calendar_agenda_empty()} />
  </div>
{:else}
  <ol class="agenda">
    {#each groups as group (dayKey(group.day))}
      <li class="day" data-today={sameDay(group.day, today) || undefined}>
        <h3>{dayHeading(group.day)}</h3>
        <ul>
          {#each group.events as event (event.id)}
            <li>
              <button
                type="button"
                class="row"
                data-selected={selected === event.id || undefined}
                style:--event-color={safeColor(event.occurrence.color)}
                onclick={() => onselect(event)}
              >
                <span class="time">
                  {event.allDay ? m.calendar_all_day() : timeLabel(event.start)}
                </span>
                <span class="swatch" aria-hidden="true"></span>
                <span class="title">{event.occurrence.summary || m.calendar_untitled()}</span>
                {#if event.occurrence.location}
                  <span class="place">{event.occurrence.location}</span>
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      </li>
    {/each}
  </ol>
{/if}

<style>
  .empty {
    display: grid;
    block-size: 100%;
    place-items: center;
  }

  .agenda {
    display: grid;
    gap: 16px;
    margin: 0;
    padding: 16px 24px;
    overflow-y: auto;
    block-size: 100%;
    box-sizing: border-box;
    list-style: none;
    color: var(--color-text);
  }

  h3 {
    margin: 0 0 4px;
    color: var(--color-muted);
    font-size: var(--text-label);
    font-weight: 600;
  }

  .day[data-today] h3 {
    color: var(--color-accent);
  }

  ul {
    display: grid;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: grid;
    grid-template-columns: 84px 4px minmax(0, 1fr) auto;
    align-items: center;
    gap: 12px;
    inline-size: 100%;
    min-block-size: 36px;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-row);
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: var(--text-body);
    text-align: start;
    cursor: pointer;
  }

  .row:hover {
    background: var(--row-hover);
  }

  .row[data-selected] {
    background: var(--row-active);
  }

  .row:focus-visible {
    outline: 2px solid var(--color-ring);
  }

  .time {
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  .swatch {
    align-self: stretch;
    border-radius: var(--radius-capsule);
    background: var(--event-color);
  }

  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .place {
    max-inline-size: 220px;
    overflow: hidden;
    color: var(--color-muted);
    font-size: var(--text-label);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
