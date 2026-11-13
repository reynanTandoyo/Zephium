<!--
  The usual question for a repeating event: this event, or all events. It
  stands in for the delete confirmation, and decides what an edit or a move
  writes. Nothing is sent until one of the two is chosen.
-->
<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import Button from "$shared/ui/Button";
  import type { CalendarFault, Scope } from "$domain/calendar";
  import { faultText } from "../lib/format";

  let {
    action,
    canThis,
    pending,
    fault,
    onchoose,
    oncancel,
  }: {
    action: "edit" | "move" | "delete";
    /** The occurrence can be addressed on its own. */
    canThis: boolean;
    pending: boolean;
    fault: CalendarFault | null;
    onchoose: (scope: Scope) => void;
    oncancel: () => void;
  } = $props();

  const question = $derived(
    action === "edit"
      ? m.calendar_scope_edit()
      : action === "move"
        ? m.calendar_scope_move()
        : m.calendar_scope_delete(),
  );
</script>

<section class="scope" aria-label={question}>
  <h2>{question}</h2>
  {#if fault}<p class="error" role="alert">{faultText(fault)}</p>{/if}
  <div class="actions">
    {#if canThis}
      <Button
        variant={action === "delete" ? "danger" : "primary"}
        {pending}
        onclick={() => onchoose("this")}>{m.calendar_scope_this()}</Button
      >
    {/if}
    <Button
      variant={action === "delete" ? "danger" : canThis ? "secondary" : "primary"}
      {pending}
      onclick={() => onchoose("all")}>{m.calendar_scope_all()}</Button
    >
    <Button variant="ghost" disabled={pending} onclick={oncancel}>{m.calendar_cancel()}</Button>
  </div>
</section>

<style>
  .scope {
    display: grid;
    align-content: start;
    gap: 12px;
    padding: 16px;
    color: var(--color-text);
  }

  h2 {
    margin: 0;
    font-size: var(--text-title);
    font-weight: 600;
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
