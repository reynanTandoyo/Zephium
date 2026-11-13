<!--
  Day and week as a time grid: all-day and multi-day events as bars above,
  timed events in hour rows with overlaps side by side. A press on an empty
  slot creates (drag to size it); a drag on an event moves it. Nothing is saved
  here: the page owns the editor and the native call.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import * as m from "$shared/i18n/messages";
  import {
    dayKey,
    dayNumber,
    editPolicy,
    isBanner,
    packSegments,
    sameDay,
    timedSegments,
    type PlacedEvent,
  } from "$domain/calendar";
  import { DRAG_SLOP, SNAP_MINUTES, dayUnder, snap } from "../lib/drag";
  import {
    dayOfMonth,
    eventWhen,
    hourLabel,
    safeColor,
    timeLabel,
    weekdayShort,
    dayTitle,
  } from "../lib/format";

  let {
    days,
    events,
    writable,
    selected,
    onselect,
    oncreate,
    onmove,
  }: {
    days: readonly Date[];
    events: readonly PlacedEvent[];
    /** Some calendar accepts new events. */
    writable: boolean;
    selected: string | null;
    onselect: (event: PlacedEvent) => void;
    oncreate: (start: Date, end: Date) => void;
    onmove: (event: PlacedEvent, start: Date) => void;
  } = $props();

  const HOUR = 48;
  const hours = Array.from({ length: 24 }, (_, hour) => hour);

  const banners = $derived(
    packSegments(
      events.filter((event) => isBanner(event)),
      days,
    ),
  );
  const bannerLanes = $derived(
    banners.reduce((most, segment) => Math.max(most, segment.lane + 1), 0),
  );
  const timed = $derived(days.map((day) => ({ day, segments: timedSegments(events, day) })));

  let scroller = $state<HTMLElement>();
  let columns = $state<HTMLElement>();
  let now = $state(new Date());

  onMount(() => {
    const timer = setInterval(() => (now = new Date()), 60_000);
    // Open on the working day rather than the small hours.
    if (scroller) scroller.scrollTop = 7 * HOUR;
    return () => clearInterval(timer);
  });

  const top = (minutes: number) => (minutes / 60) * HOUR;
  const canMove = (event: PlacedEvent) => editPolicy(event.occurrence) === "edit";

  const minuteAt = (clientY: number) => {
    const box = columns!.getBoundingClientRect();
    return Math.min(1440, Math.max(0, snap(((clientY - box.top) / HOUR) * 60)));
  };
  const at = (day: Date, minutes: number) =>
    new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, minutes);

  // Creating: press an empty slot, drag to size it, release to open the editor.
  let creating = $state<{ index: number; from: number; to: number; moved: boolean } | null>(null);
  let anchorMinute = 0;
  let pressedAt = 0;

  function createDown(event: PointerEvent, index: number) {
    if (!writable || event.button !== 0 || (event.target as HTMLElement).closest(".event")) return;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    anchorMinute = minuteAt(event.clientY);
    pressedAt = event.clientY;
    creating = { index, from: anchorMinute, to: anchorMinute, moved: false };
  }
  function createMove(event: PointerEvent) {
    if (!creating) return;
    const moved = creating.moved || Math.abs(event.clientY - pressedAt) > DRAG_SLOP;
    const here = minuteAt(event.clientY);
    creating = {
      ...creating,
      moved,
      from: Math.min(anchorMinute, here),
      to: Math.max(anchorMinute, here),
    };
  }
  function createUp() {
    if (!creating) return;
    const { index, from, to, moved } = creating;
    creating = null;
    const day = days[index]!;
    // A plain click makes an hour; a drag makes what it covered, at least a step.
    const start = moved ? from : Math.min(from, 1440 - 60);
    const end = moved ? Math.max(to, from + SNAP_MINUTES) : start + 60;
    oncreate(at(day, start), at(day, Math.min(end, 1440)));
  }
  function createKey(event: KeyboardEvent) {
    if (event.key === "Escape") creating = null;
  }

  // Moving: drag an event; the day comes from the column under the pointer.
  let moving = $state<{ id: string; x: number; y: number } | null>(null);
  let press: { event: PlacedEvent; day: Date; x: number; y: number; active: boolean } | null = null;
  let suppressClick = false;

  function moveDown(pointer: PointerEvent, event: PlacedEvent, day: Date) {
    if (pointer.button !== 0 || !canMove(event)) return;
    (pointer.currentTarget as HTMLElement).setPointerCapture(pointer.pointerId);
    press = { event, day, x: pointer.clientX, y: pointer.clientY, active: false };
  }
  function moveMove(pointer: PointerEvent) {
    if (!press) return;
    const dx = pointer.clientX - press.x;
    const dy = pointer.clientY - press.y;
    if (!press.active && Math.hypot(dx, dy) < DRAG_SLOP) return;
    press.active = true;
    moving = { id: press.event.id, x: dx, y: (snap((dy / HOUR) * 60) / 60) * HOUR };
  }
  function moveUp(pointer: PointerEvent) {
    const current = press;
    press = null;
    moving = null;
    if (!current?.active) return;
    suppressClick = true;
    setTimeout(() => (suppressClick = false), 0);
    const target = dayUnder(pointer.clientX, pointer.clientY) ?? current.day;
    const minutes = snap(((pointer.clientY - current.y) / HOUR) * 60);
    const { start } = current.event;
    const shifted = new Date(
      start.getFullYear(),
      start.getMonth(),
      start.getDate() + (dayNumber(target) - dayNumber(current.day)),
      start.getHours(),
      start.getMinutes() + minutes,
    );
    if (shifted.getTime() !== start.getTime()) onmove(current.event, shifted);
  }
  const pick = (event: PlacedEvent) => {
    if (!suppressClick) onselect(event);
  };

  const label = (event: PlacedEvent) =>
    `${event.occurrence.summary || m.calendar_untitled()}, ${eventWhen(event)}`;
</script>

<div class="grid" style:--days={days.length}>
  <div class="head">
    <div class="gutter"></div>
    {#each days as day (dayKey(day))}
      <div class="day-head" data-today={sameDay(day, now) || undefined}>
        <span class="weekday">{weekdayShort(day)}</span>
        <span class="number">{dayOfMonth(day)}</span>
      </div>
    {/each}
  </div>

  {#if bannerLanes > 0}
    <div class="banners" style:--lanes={bannerLanes}>
      <div class="gutter"></div>
      <div class="banner-area">
        {#each days as day, index (dayKey(day))}
          <div class="banner-column" data-day={dayKey(day)} style:grid-column={index + 1}></div>
        {/each}
        {#each banners as segment (segment.event.id)}
          {@const event = segment.event}
          <button
            type="button"
            class="chip"
            data-selected={selected === event.id || undefined}
            data-continues-before={segment.continuesBefore || undefined}
            data-continues-after={segment.continuesAfter || undefined}
            style:grid-column="{segment.column + 1} / span {segment.span}"
            style:grid-row={segment.lane + 1}
            style:--event-color={safeColor(event.occurrence.color)}
            aria-label={label(event)}
            onclick={() => pick(event)}
          >
            {event.occurrence.summary || m.calendar_untitled()}
          </button>
        {/each}
      </div>
    </div>
  {/if}

  <div class="scroll" bind:this={scroller}>
    <div class="body">
      <div class="gutter hours" aria-hidden="true">
        {#each hours as hour (hour)}
          <div class="hour-label" style:top="{top(hour * 60)}px">
            {hour === 0 ? "" : hourLabel(hour)}
          </div>
        {/each}
      </div>
      <div class="columns" bind:this={columns}>
        {#each timed as { day, segments }, index (dayKey(day))}
          <!-- Pointer-only slot creation: keyboard users have the New event button and N. -->
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <div
            class="column"
            role="group"
            aria-label={dayTitle(day)}
            data-day={dayKey(day)}
            data-today={sameDay(day, now) || undefined}
            onpointerdown={(event) => createDown(event, index)}
            onpointermove={createMove}
            onpointerup={createUp}
            onpointercancel={() => (creating = null)}
            onkeydown={createKey}
          >
            {#each hours as hour (hour)}
              <div class="line" style:top="{top(hour * 60)}px"></div>
            {/each}
            {#if creating && creating.index === index}
              <div
                class="slot"
                style:top="{top(creating.from)}px"
                style:height="{Math.max(top(creating.to - creating.from), 4)}px"
              ></div>
            {/if}
            {#each segments as segment (segment.event.id)}
              {@const event = segment.event}
              {@const dragged = moving?.id === event.id}
              <button
                type="button"
                class="event"
                data-selected={selected === event.id || undefined}
                data-dragging={dragged || undefined}
                data-movable={canMove(event) || undefined}
                data-continues-before={segment.continuesBefore || undefined}
                data-continues-after={segment.continuesAfter || undefined}
                style:top="{top(segment.top)}px"
                style:height="{top(segment.bottom - segment.top)}px"
                style:inset-inline-start="{(segment.column / segment.columns) * 100}%"
                style:inline-size="{(1 / segment.columns) * 100}%"
                style:--event-color={safeColor(event.occurrence.color)}
                style:transform={dragged ? `translate(${moving!.x}px, ${moving!.y}px)` : undefined}
                aria-label={label(event)}
                onpointerdown={(pointer) => moveDown(pointer, event, day)}
                onpointermove={moveMove}
                onpointerup={moveUp}
                onpointercancel={() => {
                  press = null;
                  moving = null;
                }}
                onclick={() => pick(event)}
              >
                <span class="title">{event.occurrence.summary || m.calendar_untitled()}</span>
                {#if segment.bottom - segment.top >= 45}
                  <span class="time">{timeLabel(event.start)}</span>
                {/if}
              </button>
            {/each}
            {#if sameDay(day, now)}
              <div class="now" style:top="{top(now.getHours() * 60 + now.getMinutes())}px"></div>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  </div>
</div>

<style>
  .grid {
    --gutter: 56px;

    display: flex;
    flex-direction: column;
    min-block-size: 0;
    block-size: 100%;
    color: var(--color-text);
  }

  .gutter {
    flex: none;
    inline-size: var(--gutter);
  }

  .head,
  .banners {
    display: flex;
    flex: none;
    border-block-end: 1px solid var(--color-border);
    overflow-y: hidden;
    scrollbar-gutter: stable;
  }

  .day-head {
    flex: 1 1 0;
    display: flex;
    align-items: baseline;
    justify-content: center;
    gap: 6px;
    min-inline-size: 0;
    padding-block: 8px;
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  .number {
    display: inline-grid;
    place-items: center;
    min-inline-size: 26px;
    block-size: 26px;
    border-radius: var(--radius-capsule);
    color: var(--color-text);
    font-size: var(--text-body);
    font-weight: 600;
  }

  .day-head[data-today] .number {
    background: var(--color-accent);
    color: var(--color-on-accent);
  }

  .banner-area {
    display: grid;
    flex: 1 1 0;
    grid-template-columns: repeat(var(--days), minmax(0, 1fr));
    grid-auto-rows: 22px;
    gap: 2px 0;
    min-inline-size: 0;
    padding-block: 4px;
  }

  .banner-column {
    grid-row: 1 / span var(--lanes);
    border-inline-start: 1px solid var(--color-border);
  }

  .chip {
    z-index: 1;
    overflow: hidden;
    margin-inline: 2px;
    padding: 0 8px;
    border: 0;
    border-inline-start: 3px solid var(--event-color);
    border-radius: var(--radius-inset);
    background: color-mix(in srgb, var(--event-color) 24%, var(--color-surface));
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-caption);
    line-height: 22px;
    text-align: start;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }

  .chip[data-continues-before] {
    margin-inline-start: 0;
    border-start-start-radius: 0;
    border-end-start-radius: 0;
  }

  .chip[data-continues-after] {
    margin-inline-end: 0;
    border-start-end-radius: 0;
    border-end-end-radius: 0;
  }

  .scroll {
    flex: 1 1 0;
    overflow-y: auto;
    scrollbar-gutter: stable;
    min-block-size: 0;
  }

  .body {
    display: flex;
    position: relative;
  }

  .hours {
    position: relative;
    block-size: calc(24 * 48px);
  }

  .hour-label {
    position: absolute;
    inset-inline-end: 8px;
    transform: translateY(-50%);
    color: var(--color-faint);
    font-size: var(--text-caption);
    white-space: nowrap;
  }

  .columns {
    display: flex;
    flex: 1 1 0;
    min-inline-size: 0;
  }

  .column {
    position: relative;
    flex: 1 1 0;
    min-inline-size: 0;
    block-size: calc(24 * 48px);
    border-inline-start: 1px solid var(--color-border);
    touch-action: pan-y;
    user-select: none;
  }

  .column[data-today] {
    background: color-mix(in srgb, var(--color-accent) 5%, transparent);
  }

  .line {
    position: absolute;
    inset-inline: 0;
    block-size: 0;
    border-block-start: 1px solid var(--color-border);
    pointer-events: none;
  }

  .slot {
    position: absolute;
    inset-inline: 2px;
    border-radius: var(--radius-inset);
    background: var(--color-accent-soft);
    outline: 1px solid var(--color-accent);
    pointer-events: none;
  }

  .event {
    position: absolute;
    display: flex;
    flex-direction: column;
    gap: 1px;
    overflow: hidden;
    box-sizing: border-box;
    padding: 3px 6px;
    border: 0;
    border-inline-start: 3px solid var(--event-color);
    border-radius: var(--radius-inset);
    background: color-mix(in srgb, var(--event-color) 24%, var(--color-surface));
    box-shadow: 0 0 0 1px var(--color-surface);
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-caption);
    text-align: start;
    cursor: pointer;
    touch-action: none;
  }

  .event[data-movable] {
    cursor: grab;
  }

  .event[data-dragging] {
    z-index: 3;
    box-shadow: var(--shadow-card);
    cursor: grabbing;
  }

  .event[data-continues-before] {
    border-start-start-radius: 0;
  }

  .event[data-continues-after] {
    border-end-start-radius: 0;
  }

  .title {
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .time {
    color: var(--color-muted);
  }

  .event[data-selected],
  .chip[data-selected] {
    outline: 2px solid var(--color-ring);
    outline-offset: 0;
  }

  .event:focus-visible,
  .chip:focus-visible {
    outline: 2px solid var(--color-ring);
  }

  .now {
    position: absolute;
    inset-inline: 0;
    z-index: 2;
    block-size: 0;
    border-block-start: 2px solid var(--color-danger);
    pointer-events: none;
  }

  .now::before {
    content: "";
    position: absolute;
    inset-inline-start: -4px;
    inset-block-start: -5px;
    inline-size: 8px;
    block-size: 8px;
    border-radius: var(--radius-capsule);
    background: var(--color-danger);
  }

  @media (forced-colors: active) {
    .event,
    .chip {
      border: 1px solid CanvasText;
    }
  }
</style>
