<!--
  The rest of today's events, a few, in the Time panel: when each starts, a
  Join button for a meeting about to begin, and a click that opens Calendar on
  the event. It reads nothing until it is shown, and says nothing without a
  calendar account.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import * as m from "$shared/i18n/messages";
  import { getLocale } from "$shared/i18n/runtime";
  import Button from "$shared/ui/Button";
  import { openMeeting, safeColor, today } from "$domain/calendar-agenda";
  import { eventOpen, linkOf } from "$domain/calendar-link";

  const LIMIT = 4;

  onMount(() => today.acquire());

  const items = $derived(today.upcoming());
  const shown = $derived(items.slice(0, LIMIT));
  const clock = (date: Date) =>
    new Intl.DateTimeFormat(getLocale(), { hour: "numeric", minute: "2-digit" }).format(date);
  const when = (event: (typeof items)[number]) =>
    event.allDay
      ? m.calendar_all_day()
      : event.start.getTime() <= Date.now()
        ? m.calendar_now()
        : clock(event.start);
</script>

{#if today.available()}
  <section class="today" aria-label={m.calendar_today_events()}>
    <h3>{m.calendar_today_events()}</h3>
    {#if shown.length === 0}
      <p class="none">{m.calendar_nothing_more()}</p>
    {:else}
      <ul>
        {#each shown as event (event.id)}
          {@const join = today.joinable(event)}
          <li>
            <button
              type="button"
              class="row"
              style:--event-color={safeColor(event.occurrence.color)}
              aria-label={m.calendar_open_event({ title: event.occurrence.summary })}
              onclick={() => eventOpen.open(linkOf(event.occurrence))}
            >
              <span class="when">{when(event)}</span>
              <span class="title">{event.occurrence.summary || m.calendar_untitled()}</span>
            </button>
            {#if join}
              <Button size="compact" variant="primary" onclick={() => void openMeeting(join.url)}
                >{m.calendar_join()}</Button
              >
            {/if}
          </li>
        {/each}
      </ul>
      {#if items.length > LIMIT}
        <p class="none">{m.calendar_more({ count: items.length - LIMIT })}</p>
      {/if}
    {/if}
  </section>
{/if}

<style>
  .today {
    display: grid;
    gap: 6px;
    color: var(--color-text);
  }

  h3 {
    margin: 0;
    color: var(--color-muted);
    font-size: var(--text-label);
    font-weight: 600;
  }

  ul {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .row {
    display: grid;
    flex: 1;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: center;
    gap: 8px;
    min-inline-size: 0;
    min-block-size: 28px;
    padding: 2px 8px;
    border: 0;
    border-inline-start: 3px solid var(--event-color);
    border-radius: var(--radius-inset);
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: var(--text-label);
    text-align: start;
    cursor: pointer;
  }

  .row:hover {
    background: var(--row-hover);
  }

  .row:focus-visible {
    outline: 2px solid var(--color-ring);
  }

  .when {
    color: var(--color-muted);
    font-variant-numeric: tabular-nums;
  }

  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .none {
    margin: 0;
    color: var(--color-muted);
    font-size: var(--text-label);
  }
</style>
