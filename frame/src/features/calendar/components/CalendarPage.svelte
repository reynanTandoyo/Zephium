<!--
  Calendar as a full-window mode: a sidebar with the month navigator and the
  calendars, the day, week, month or agenda view in the middle, and one panel on
  the side for an event's details, the editor, or the accounts. State owned by
  Rust is read through `$domain/calendar`; this page keeps only the view, the
  date and which panel is open.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import {
    Add01Icon,
    ArrowLeft01Icon,
    ArrowRight01Icon,
    Calendar01Icon,
    RefreshIcon,
    Settings02Icon,
  } from "@hugeicons/core-free-icons";
  import * as m from "$shared/i18n/messages";
  import Button from "$shared/ui/Button";
  import EmptyState from "$shared/ui/EmptyState";
  import Icon from "$shared/ui/Icon";
  import IconButton from "$shared/ui/IconButton";
  import SegmentedControl from "$shared/ui/SegmentedControl";
  import {
    calendar,
    editPolicy,
    formForSlot,
    formFromEvent,
    formToDraft,
    movedForm,
    placeAll,
    rangeOf,
    startOfDay,
    stepAnchor,
    visibleDays,
    type CalendarFault,
    type CalendarView,
    type EventForm,
    type PlacedEvent,
    type Scope,
  } from "$domain/calendar";
  import type { CalendarEventDraftV1 } from "$shared/ipc/bindings";
  import { firstDayOfWeek } from "$domain/time";
  import AccountsPanel from "./AccountsPanel.svelte";
  import AgendaList from "./AgendaList.svelte";
  import CalendarList from "./CalendarList.svelte";
  import EventDetails from "./EventDetails.svelte";
  import EventEditor from "./EventEditor.svelte";
  import MiniMonth from "./MiniMonth.svelte";
  import MonthGrid from "./MonthGrid.svelte";
  import ScopeChoice from "./ScopeChoice.svelte";
  import TimeGrid from "./TimeGrid.svelte";
  import { faultText, viewTitle } from "../lib/format";

  type Panel =
    | { kind: "details"; id: string }
    | { kind: "editor"; editing: PlacedEvent | null; scope: Scope | null; form: EventForm }
    | { kind: "scope"; action: "edit" | "move" | "delete"; event: PlacedEvent; start: Date | null }
    | { kind: "accounts" };

  // Weeks start where the locale says (1 is Monday … 7 is Sunday); the date
  // maths counts from Sunday as 0.
  const firstDay = firstDayOfWeek() % 7;

  let view = $state<CalendarView>("week");
  let anchor = $state(startOfDay(new Date()));
  let panel = $state<Panel | null>(null);
  let panelFault = $state<CalendarFault | null>(null);
  let moveFault = $state<CalendarFault | null>(null);

  onMount(() => {
    void calendar.init();
    return () => calendar.dispose();
  });

  const days = $derived(visibleDays(view, anchor, firstDay));
  $effect(() => {
    const range = rangeOf(view, anchor, firstDay);
    void calendar.setRange(range.fromText, range.toText);
  });

  const placed = $derived(placeAll(calendar.occurrences()));
  const writable = $derived(calendar.writableCalendars());
  const calendarName = (id: string) =>
    calendar.calendars().find((entry) => entry.id === id)?.name ?? "";

  const shown = $derived.by(() => {
    if (panel?.kind !== "details") return null;
    const id = panel.id;
    return placed.find((event) => event.id === id) ?? null;
  });
  const selectedId = $derived(panel?.kind === "details" ? panel.id : null);
  const open = $derived(panel !== null && (panel.kind !== "details" || shown !== null));
  const empty = $derived(calendar.listState() === "ready" && calendar.accounts().length === 0);

  const title = $derived(viewTitle(view, days, anchor));
  const viewOptions = $derived([
    { value: "day", label: m.calendar_view_day() },
    { value: "week", label: m.calendar_view_week() },
    { value: "month", label: m.calendar_view_month() },
    { value: "agenda", label: m.calendar_view_agenda() },
  ]);

  const go = (day: Date) => (anchor = startOfDay(day));
  const step = (direction: 1 | -1) => (anchor = stepAnchor(view, anchor, direction));
  const today = () => go(new Date());

  function close() {
    panel = null;
    panelFault = null;
  }

  function show(event: PlacedEvent) {
    panelFault = null;
    panel = { kind: "details", id: event.id };
  }

  const defaultCalendar = () =>
    (writable.find((entry) => entry.visible) ?? writable[0])?.id ?? null;

  function draftEvent(start: Date, end: Date) {
    const calendarId = defaultCalendar();
    if (!calendarId) return;
    panelFault = null;
    panel = {
      kind: "editor",
      editing: null,
      scope: null,
      form: formForSlot(calendarId, start, end),
    };
  }

  /** An hour starting at the next full hour, on the day being viewed. */
  function createNow() {
    const day = startOfDay(anchor);
    const hour = Math.min(22, new Date().getHours() + 1);
    const start = new Date(day.getFullYear(), day.getMonth(), day.getDate(), hour);
    draftEvent(start, new Date(day.getFullYear(), day.getMonth(), day.getDate(), hour + 1));
  }

  function createOnDay(day: Date) {
    const start = new Date(day.getFullYear(), day.getMonth(), day.getDate(), 9);
    draftEvent(start, new Date(start.getTime() + 3_600_000));
  }

  const repeats = (event: PlacedEvent) => event.occurrence.recurring;

  /** A repeating event asks which events first; a single one goes straight on. */
  function ask(action: "edit" | "move" | "delete", event: PlacedEvent, start: Date | null = null) {
    if (editPolicy(event.occurrence) !== "edit") return;
    panelFault = null;
    panel = { kind: "scope", action, event, start };
  }

  function openEditor(event: PlacedEvent, scope: Scope | null) {
    panelFault = null;
    panel = { kind: "editor", editing: event, scope, form: formFromEvent(event, scope ?? "this") };
  }

  function edit(event: PlacedEvent) {
    if (editPolicy(event.occurrence) !== "edit") return;
    if (repeats(event)) ask("edit", event);
    else openEditor(event, null);
  }

  /** One native write for an existing event: the occurrence when the scope is
   *  `this`, otherwise the event (the whole series when it repeats). */
  function write(event: PlacedEvent, scope: Scope | null, draft: CalendarEventDraftV1) {
    const { calendar_id: id, uid, recurrence_id: instance } = event.occurrence;
    return scope === "this" && instance !== null
      ? calendar.updateOccurrence(id, uid, instance, draft)
      : calendar.updateEvent(id, uid, draft);
  }

  async function save(calendarId: string, draft: CalendarEventDraftV1) {
    const current = panel;
    if (current?.kind !== "editor") return;
    panelFault = null;
    const result = current.editing
      ? await write(current.editing, current.scope, draft)
      : await calendar.createEvent(calendarId, draft);
    if (!result.ok) {
      panelFault = result.error;
      return;
    }
    if (panel === current) close();
  }

  async function remove(event: PlacedEvent, scope: Scope | null) {
    panelFault = null;
    const { calendar_id: id, uid, recurrence_id: instance } = event.occurrence;
    const result =
      scope === "this" && instance !== null
        ? await calendar.deleteOccurrence(id, uid, instance)
        : await calendar.deleteEvent(id, uid);
    if (!result.ok) {
      panelFault = result.error;
      return;
    }
    close();
  }

  async function performMove(event: PlacedEvent, start: Date, scope: Scope | null) {
    moveFault = null;
    const result = formToDraft(movedForm(event, start, scope ?? "this"));
    if (!result.ok) {
      moveFault = "invalid_input";
      return;
    }
    const saved = await write(event, scope, result.draft);
    if (!saved.ok) moveFault = saved.error;
  }

  function move(event: PlacedEvent, start: Date) {
    if (repeats(event)) ask("move", event, start);
    else void performMove(event, start, null);
  }

  function choose(scope: Scope) {
    const current = panel;
    if (current?.kind !== "scope") return;
    if (current.action === "edit") openEditor(current.event, scope);
    else if (current.action === "delete") void remove(current.event, scope);
    else if (current.start) {
      close();
      void performMove(current.event, current.start, scope);
    }
  }

  async function join(url: string) {
    panelFault = (await calendar.openLink(url)) ? null : "unavailable";
  }

  function chooseView(next: string) {
    view = next as CalendarView;
  }

  const SHORTCUTS: Record<string, CalendarView> = { d: "day", w: "week", m: "month", a: "agenda" };

  function keydown(event: KeyboardEvent) {
    if (event.defaultPrevented) return;
    if (event.key === "Escape" && panel) {
      event.preventDefault();
      close();
      return;
    }
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const target = event.target as HTMLElement | null;
    // Fields keep every key. Groups that use the arrows themselves keep those.
    if (target?.closest("input, textarea, select, [contenteditable]")) return;
    const arrow = event.key.startsWith("Arrow");
    if (
      arrow &&
      target?.closest("[role='radiogroup'], [role='listbox'], [role='combobox'], .panel")
    )
      return;
    const key = event.key.toLowerCase();
    if (event.key === "ArrowLeft" || event.key === "ArrowUp") step(-1);
    else if (event.key === "ArrowRight" || event.key === "ArrowDown") step(1);
    else if (key === "t") today();
    else if (key === "n" && writable.length > 0) createNow();
    else if (key in SHORTCUTS) view = SHORTCUTS[key]!;
    else return;
    event.preventDefault();
  }

  const loading = $derived(calendar.eventState() === "loading" || calendar.busy() === "sync");
</script>

<svelte:window onkeydown={keydown} />

<section class="page" aria-label={m.mode_calendar()}>
  <aside class="sidebar">
    <MiniMonth {anchor} {view} {firstDay} onpick={go} />
    <CalendarList />
    <div class="side-actions">
      <Button variant="ghost" onclick={() => (panel = { kind: "accounts" })}>
        <Icon icon={Settings02Icon} size={14} />
        {m.calendar_accounts()}
      </Button>
    </div>
  </aside>

  <div class="main">
    <header class="toolbar">
      <Button onclick={today}>{m.calendar_today()}</Button>
      <IconButton icon={ArrowLeft01Icon} label={m.calendar_previous()} onclick={() => step(-1)} />
      <IconButton icon={ArrowRight01Icon} label={m.calendar_next()} onclick={() => step(1)} />
      <h1>{title}</h1>
      <span class="status" role="status">{loading ? m.calendar_loading() : ""}</span>
      <SegmentedControl
        label={m.calendar_views()}
        options={viewOptions}
        value={view}
        size="compact"
        onchange={chooseView}
      />
      <IconButton
        icon={RefreshIcon}
        label={m.calendar_sync()}
        pending={calendar.busy() === "sync"}
        disabled={calendar.accounts().length === 0}
        onclick={() => void calendar.sync()}
      />
      <Button variant="primary" disabled={writable.length === 0} onclick={createNow}>
        <Icon icon={Add01Icon} size={14} />
        {m.calendar_new_event()}
      </Button>
    </header>

    {#if calendar.listState() === "failed"}
      <div class="banner" role="alert">
        <span>{m.calendar_load_failed()} {faultText(calendar.listFault()!)}</span>
        <Button size="compact" onclick={() => void calendar.retry()}>{m.calendar_retry()}</Button>
      </div>
    {/if}
    {#if calendar.eventState() === "failed"}
      <div class="banner" role="alert">
        <span>{m.calendar_load_failed()} {faultText(calendar.eventFault()!)}</span>
        <Button size="compact" onclick={() => void calendar.retry()}>{m.calendar_retry()}</Button>
      </div>
    {/if}
    {#if calendar.syncFailures().length > 0}
      <div class="banner" role="alert">
        {m.calendar_sync_failed({ count: calendar.syncFailures().length })}
      </div>
    {/if}
    {#if moveFault}
      <div class="banner" role="alert">{faultText(moveFault)}</div>
    {/if}
    {#if calendar.truncated()}
      <div class="banner" role="status">{m.calendar_truncated()}</div>
    {/if}
    {#if writable.length === 0 && !empty && calendar.listState() === "ready"}
      <div class="banner" role="status">{m.calendar_no_writable()}</div>
    {/if}

    <div class="view">
      {#if empty}
        <EmptyState title={m.calendar_no_accounts_title()} description={m.calendar_no_accounts()}>
          {#snippet icon()}<Icon icon={Calendar01Icon} size={28} />{/snippet}
          {#snippet action()}
            <Button variant="primary" onclick={() => (panel = { kind: "accounts" })}>
              {m.calendar_accounts()}
            </Button>
          {/snippet}
        </EmptyState>
      {:else if view === "month"}
        <MonthGrid
          {days}
          month={anchor}
          events={placed}
          writable={writable.length > 0}
          selected={selectedId}
          onselect={show}
          oncreate={createOnDay}
          onmove={move}
          onopenday={(day) => {
            go(day);
            view = "day";
          }}
        />
      {:else if view === "agenda"}
        <AgendaList {days} events={placed} selected={selectedId} onselect={show} />
      {:else}
        <TimeGrid
          {days}
          events={placed}
          writable={writable.length > 0}
          selected={selectedId}
          onselect={show}
          oncreate={draftEvent}
          onmove={move}
        />
      {/if}
    </div>
  </div>

  {#if open && panel}
    <aside class="panel">
      {#if panel.kind === "details" && shown}
        <EventDetails
          event={shown}
          calendarName={calendarName(shown.occurrence.calendar_id)}
          pending={calendar.busy() === "delete"}
          fault={panelFault}
          onedit={() => edit(shown)}
          ondelete={() => (repeats(shown) ? ask("delete", shown) : void remove(shown, null))}
          onjoin={(url) => void join(url)}
          onclose={close}
        />
      {:else if panel.kind === "editor"}
        {#key `${panel.editing?.id ?? "new"}:${panel.scope}`}
          <EventEditor
            initial={panel.form}
            editing={panel.editing !== null}
            scope={panel.scope}
            calendars={writable}
            pending={calendar.busy() === "save"}
            fault={panelFault}
            onsave={(id, draft) => void save(id, draft)}
            oncancel={close}
          />
        {/key}
      {:else if panel.kind === "scope"}
        <ScopeChoice
          action={panel.action}
          canThis={panel.event.occurrence.recurrence_id !== null}
          pending={calendar.busy() !== null}
          fault={panelFault}
          onchoose={choose}
          oncancel={close}
        />
      {:else if panel.kind === "accounts"}
        <AccountsPanel onclose={close} />
      {/if}
    </aside>
  {/if}
</section>

<style>
  .page {
    display: flex;
    block-size: 100%;
    min-block-size: 0;
    color: var(--color-text);
  }

  .sidebar {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: 20px;
    inline-size: 232px;
    padding: 16px;
    overflow-y: auto;
    border-inline-end: 1px solid var(--color-border);
  }

  .side-actions {
    margin-block-start: auto;
  }

  .main {
    display: flex;
    flex: 1 1 0;
    flex-direction: column;
    min-inline-size: 0;
  }

  .toolbar {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
  }

  h1 {
    flex: 1;
    min-inline-size: 0;
    margin: 0 8px;
    overflow: hidden;
    font-size: var(--text-title);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .status {
    color: var(--color-muted);
    font-size: var(--text-label);
  }

  .banner {
    display: flex;
    flex: none;
    align-items: center;
    gap: 12px;
    margin: 0 16px 8px;
    padding: 6px 12px;
    border-radius: var(--radius-row);
    background: var(--color-fill);
    color: var(--color-text);
    font-size: var(--text-label);
  }

  .banner[role="alert"] {
    color: var(--color-danger);
  }

  .view {
    flex: 1 1 0;
    min-block-size: 0;
    overflow: hidden;
  }

  .panel {
    flex: none;
    inline-size: 320px;
    overflow-y: auto;
    border-inline-start: 1px solid var(--color-border);
    background: var(--color-surface);
  }
</style>
