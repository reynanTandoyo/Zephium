<!--
  A small month to jump by: pick a day and the page goes there. It has its own
  month, so browsing it does not move the view until a day is chosen.
-->
<script lang="ts">
  import { ArrowLeft01Icon, ArrowRight01Icon } from "@hugeicons/core-free-icons";
  import * as m from "$shared/i18n/messages";
  import IconButton from "$shared/ui/IconButton";
  import {
    dayKey,
    monthGrid,
    sameDay,
    stepAnchor,
    visibleDays,
    type CalendarView,
  } from "$domain/calendar";
  import { dayOfMonth, dayTitle, monthTitle, weekdayShort } from "../lib/format";

  let {
    anchor,
    view,
    firstDay,
    onpick,
  }: {
    anchor: Date;
    view: CalendarView;
    firstDay: number;
    onpick: (day: Date) => void;
  } = $props();

  // Browsing is this month's own until the page moves, which brings it back.
  let browsed = $state<Date | null>(null);
  $effect(() => {
    void anchor;
    browsed = null;
  });
  const shown = $derived(browsed ?? new Date(anchor.getFullYear(), anchor.getMonth(), 1));

  const grid = $derived(monthGrid(shown, firstDay));
  const covered = $derived(new Set(visibleDays(view, anchor, firstDay).map(dayKey)));
  const today = new Date();
</script>

<section class="mini" aria-label={m.calendar_mini_label()}>
  <header>
    <h2>{monthTitle(shown)}</h2>
    <IconButton
      icon={ArrowLeft01Icon}
      label={m.calendar_previous()}
      size={14}
      buttonSize={24}
      onclick={() => (browsed = stepAnchor("month", shown, -1))}
    />
    <IconButton
      icon={ArrowRight01Icon}
      label={m.calendar_next()}
      size={14}
      buttonSize={24}
      onclick={() => (browsed = stepAnchor("month", shown, 1))}
    />
  </header>
  <div class="days">
    {#each grid.slice(0, 7) as day (dayKey(day))}
      <span class="weekday" aria-hidden="true">{weekdayShort(day).slice(0, 2)}</span>
    {/each}
    {#each grid as day (dayKey(day))}
      <button
        type="button"
        class="day"
        aria-label={dayTitle(day)}
        aria-current={sameDay(day, today) ? "date" : undefined}
        data-outside={day.getMonth() !== shown.getMonth() || undefined}
        data-covered={covered.has(dayKey(day)) || undefined}
        onclick={() => onpick(day)}
      >
        {dayOfMonth(day)}
      </button>
    {/each}
  </div>
</section>

<style>
  .mini {
    display: grid;
    gap: 6px;
    color: var(--color-text);
  }

  header {
    display: flex;
    align-items: center;
  }

  h2 {
    flex: 1;
    margin: 0;
    font-size: var(--text-label);
    font-weight: 600;
  }

  .days {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 2px 0;
    text-align: center;
  }

  .weekday {
    color: var(--color-faint);
    font-size: var(--text-caption);
  }

  .day {
    display: grid;
    place-items: center;
    block-size: 26px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-capsule);
    background: transparent;
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-caption);
    cursor: pointer;
  }

  .day[data-outside] {
    color: var(--color-faint);
  }

  .day[data-covered] {
    background: var(--color-fill);
  }

  .day:hover {
    background: var(--color-fill-hover);
  }

  .day[aria-current="date"] {
    background: var(--color-accent);
    color: var(--color-on-accent);
    font-weight: 600;
  }

  .day:focus-visible {
    outline: 2px solid var(--color-ring);
  }
</style>
