<!--
  Month as rows of weeks. Every event is a bar across the days it covers, packed
  into lanes; a day with more bars than fit says how many are hidden. Press an
  empty day to create an event there; drag a bar onto another day to move it.
-->
<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import {
    dayKey,
    dayNumber,
    editPolicy,
    packSegments,
    sameDay,
    type PlacedEvent,
  } from "$domain/calendar";
  import { DRAG_SLOP, dayUnder } from "../lib/drag";
  import {
    dayOfMonth,
    dayTitle,
    eventWhen,
    safeColor,
    timeLabel,
    weekdayShort,
  } from "../lib/format";

  let {
    days,
    month,
    events,
    writable,
    selected,
    onselect,
    oncreate,
    onmove,
    onopenday,
  }: {
    days: readonly Date[];
    /** Any date in the month shown; days outside it are dimmed. */
    month: Date;
    events: readonly PlacedEvent[];
    writable: boolean;
    selected: string | null;
    onselect: (event: PlacedEvent) => void;
    /** An empty day was pressed: a whole-day slot. */
    oncreate: (day: Date) => void;
    onmove: (event: PlacedEvent, start: Date) => void;
    onopenday: (day: Date) => void;
  } = $props();

  const LANES = 3;
  let now = $state(new Date());
  const weeks = $derived(
    Array.from({ length: days.length / 7 }, (_, index) => {
      const row = days.slice(index * 7, index * 7 + 7);
      const segments = packSegments(events, row);
      const hidden = row.map((_, column) => {
        const covering = segments.filter(
          (segment) => segment.column <= column && column < segment.column + segment.span,
        );
        return covering.filter((segment) => segment.lane >= LANES).length;
      });
      return { row, segments: segments.filter((segment) => segment.lane < LANES), hidden };
    }),
  );

  let moving = $state<{ id: string; x: number; y: number } | null>(null);
  let press: { event: PlacedEvent; day: Date; x: number; y: number; active: boolean } | null = null;
  let suppressClick = false;

  const canMove = (event: PlacedEvent) => editPolicy(event.occurrence) === "edit";

  /** The day of the week row a press landed on, so a bar spanning several
   *  days moves by how far it was dragged, not from its first day. */
  function dayPressed(pointer: PointerEvent, row: readonly Date[]): Date {
    const week = (pointer.currentTarget as HTMLElement).parentElement!;
    const box = week.getBoundingClientRect();
    const index = Math.min(
      6,
      Math.max(0, Math.floor(((pointer.clientX - box.left) / box.width) * 7)),
    );
    return row[getComputedStyle(week).direction === "rtl" ? 6 - index : index]!;
  }

  function down(pointer: PointerEvent, event: PlacedEvent, row: readonly Date[]) {
    if (pointer.button !== 0 || !canMove(event)) return;
    const day = dayPressed(pointer, row);
    (pointer.currentTarget as HTMLElement).setPointerCapture(pointer.pointerId);
    press = { event, day, x: pointer.clientX, y: pointer.clientY, active: false };
  }
  function drag(pointer: PointerEvent) {
    if (!press) return;
    const dx = pointer.clientX - press.x;
    const dy = pointer.clientY - press.y;
    if (!press.active && Math.hypot(dx, dy) < DRAG_SLOP) return;
    press.active = true;
    moving = { id: press.event.id, x: dx, y: dy };
  }
  function up(pointer: PointerEvent) {
    const current = press;
    press = null;
    moving = null;
    if (!current?.active) return;
    suppressClick = true;
    setTimeout(() => (suppressClick = false), 0);
    const target = dayUnder(pointer.clientX, pointer.clientY);
    if (!target) return;
    const by = dayNumber(target) - dayNumber(current.day);
    if (by === 0) return;
    const { start } = current.event;
    onmove(
      current.event,
      new Date(
        start.getFullYear(),
        start.getMonth(),
        start.getDate() + by,
        start.getHours(),
        start.getMinutes(),
      ),
    );
  }
  const pick = (event: PlacedEvent) => {
    if (!suppressClick) onselect(event);
  };
</script>

<div class="month">
  <div class="weekdays" aria-hidden="true">
    {#each days.slice(0, 7) as day (dayKey(day))}
      <div>{weekdayShort(day)}</div>
    {/each}
  </div>
  <div class="weeks">
    {#each weeks as week (dayKey(week.row[0]!))}
      <div class="week">
        {#each week.row as day, column (dayKey(day))}
          <!-- Pointer-only day creation: keyboard users have the New event button and N. -->
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
          <div
            class="cell"
            role="group"
            aria-label={dayTitle(day)}
            data-day={dayKey(day)}
            data-outside={day.getMonth() !== month.getMonth() || undefined}
            data-today={sameDay(day, now) || undefined}
            style:grid-column={column + 1}
            onclick={(event) => {
              if (writable && event.target === event.currentTarget) oncreate(day);
            }}
          >
            <button
              type="button"
              class="number"
              aria-label={dayTitle(day)}
              onclick={() => onopenday(day)}>{dayOfMonth(day)}</button
            >
            {#if week.hidden[column]! > 0}
              <button type="button" class="more" onclick={() => onopenday(day)}>
                {m.calendar_more({ count: week.hidden[column]! })}
              </button>
            {/if}
          </div>
        {/each}
        {#each week.segments as segment (segment.event.id)}
          {@const event = segment.event}
          {@const dragged = moving?.id === event.id}
          <button
            type="button"
            class="bar"
            data-selected={selected === event.id || undefined}
            data-timed={!event.allDay || undefined}
            data-dragging={dragged || undefined}
            data-continues-before={segment.continuesBefore || undefined}
            data-continues-after={segment.continuesAfter || undefined}
            style:grid-column="{segment.column + 1} / span {segment.span}"
            style:grid-row={segment.lane + 2}
            style:--event-color={safeColor(event.occurrence.color)}
            style:transform={dragged ? `translate(${moving!.x}px, ${moving!.y}px)` : undefined}
            aria-label="{event.occurrence.summary || m.calendar_untitled()}, {eventWhen(event)}"
            onpointerdown={(pointer) => down(pointer, event, week.row)}
            onpointermove={drag}
            onpointerup={up}
            onpointercancel={() => {
              press = null;
              moving = null;
            }}
            onclick={() => pick(event)}
          >
            {#if !event.allDay && !segment.continuesBefore}<span class="time"
                >{timeLabel(event.start)}</span
              >{/if}
            {event.occurrence.summary || m.calendar_untitled()}
          </button>
        {/each}
      </div>
    {/each}
  </div>
</div>

<style>
  .month {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
    color: var(--color-text);
  }

  .weekdays {
    display: grid;
    flex: none;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    border-block-end: 1px solid var(--color-border);
    color: var(--color-muted);
    font-size: var(--text-label);
    text-align: center;
  }

  .weekdays > div {
    padding-block: 8px;
  }

  .weeks {
    display: grid;
    flex: 1 1 0;
    grid-auto-rows: minmax(0, 1fr);
    min-block-size: 0;
  }

  .week {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    grid-template-rows: 26px repeat(3, 20px) 1fr;
    min-block-size: 0;
    overflow: hidden;
    border-block-end: 1px solid var(--color-border);
  }

  .cell {
    grid-row: 1 / -1;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: space-between;
    min-inline-size: 0;
    padding: 2px 4px;
    border-inline-start: 1px solid var(--color-border);
  }

  .number {
    display: inline-grid;
    place-items: center;
    min-inline-size: 22px;
    block-size: 22px;
    padding: 0 4px;
    border: 0;
    border-radius: var(--radius-capsule);
    background: transparent;
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-label);
    cursor: pointer;
  }

  .number:focus-visible {
    outline: 2px solid var(--color-ring);
  }

  .cell[data-outside] .number {
    color: var(--color-faint);
  }

  .cell[data-today] .number {
    background: var(--color-accent);
    color: var(--color-on-accent);
    font-weight: 600;
  }

  .more {
    padding: 0 4px;
    border: 0;
    background: transparent;
    color: var(--color-muted);
    font: inherit;
    font-size: var(--text-caption);
    cursor: pointer;
  }

  .bar {
    z-index: 1;
    overflow: hidden;
    margin: 0 2px 2px;
    padding: 0 6px;
    border: 0;
    border-inline-start: 3px solid var(--event-color);
    border-radius: var(--radius-inset);
    background: color-mix(in srgb, var(--event-color) 24%, var(--color-surface));
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-caption);
    line-height: 18px;
    text-align: start;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
    touch-action: none;
  }

  .bar[data-timed] {
    background: transparent;
    border-inline-start-color: transparent;
  }

  .bar[data-timed]::before {
    content: "";
    display: inline-block;
    inline-size: 6px;
    block-size: 6px;
    margin-inline-end: 4px;
    border-radius: var(--radius-capsule);
    background: var(--event-color);
    vertical-align: middle;
  }

  .time {
    margin-inline-end: 4px;
    color: var(--color-muted);
  }

  .bar[data-continues-before] {
    margin-inline-start: 0;
    border-start-start-radius: 0;
    border-end-start-radius: 0;
  }

  .bar[data-continues-after] {
    margin-inline-end: 0;
    border-start-end-radius: 0;
    border-end-end-radius: 0;
  }

  .bar[data-dragging] {
    z-index: 3;
    pointer-events: none;
    background: var(--color-card);
    box-shadow: var(--shadow-card);
  }

  .bar[data-selected] {
    outline: 2px solid var(--color-ring);
  }

  .bar:focus-visible,
  .more:focus-visible {
    outline: 2px solid var(--color-ring);
  }

  @media (forced-colors: active) {
    .bar {
      border: 1px solid CanvasText;
    }
  }
</style>
