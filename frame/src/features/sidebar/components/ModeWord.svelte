<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import Icon from "$shared/ui/Icon";
  import { surface } from "$domain/surface";
  import { untrack } from "svelte";
  import { chooseMode, modeOptions } from "../lib/modes";

  let { standalone = false }: { standalone?: boolean } = $props();

  const modes = $derived(modeOptions());
  const current = $derived(surface.modeOf(surface.currentPage()));
  const index = $derived(
    Math.max(
      0,
      modes.findIndex((mode) => mode.value === current),
    ),
  );
  // The thumb stretches only on a change of side, never on arrival.
  let moved = $state(false);
  let seen: string | null = null;
  $effect(() => {
    const next = current;
    untrack(() => {
      if (seen !== null && seen !== next) moved = true;
      seen = next;
    });
  });
</script>

<!--
  The rail's modes as one small switch: a mark for each, a thumb under the one
  you are in that slides across once native has moved there, and a click on
  another goes. Names come on hover and to assistive technology.
-->
<div
  class="mode-switch"
  style:--count={modes.length}
  style:--index={index}
  data-mode={current}
  data-moved={moved}
  data-standalone={standalone}
  role="group"
  aria-label={m.ui_mode_work_hint()}
>
  <span class="thumb" aria-hidden="true"></span>
  {#each modes as mode (mode.value)}<button
      type="button"
      class="mode"
      aria-pressed={current === mode.value}
      aria-label={mode.label}
      title={mode.label}
      onclick={() => {
        if (current !== mode.value) chooseMode(mode.value);
      }}><Icon icon={mode.icon} size={15} strokeWidth={1.7} /></button
    >{/each}
</div>

<style>
  .mode-switch {
    --cell: 24px;

    position: relative;
    display: grid;
    grid-template-columns: repeat(var(--count), var(--cell));
    gap: 0;
    padding: 2px;
    border-radius: var(--radius-capsule);
    background: var(--color-fill);
    box-shadow: var(--row-rim);
  }

  /* The expanded switch's own thumb, lifted on the glass rather than laid on
     it as a disc, and it travels like liquid: drawn out along the way, round
     again on arrival. */
  .thumb {
    position: absolute;
    inset-block: 2px;
    inset-inline-start: 2px;
    inline-size: var(--cell);
    border-radius: var(--radius-capsule);
    background: var(--row-active);
    box-shadow: var(--shine-top);
    translate: calc(var(--cell) * var(--index)) 0;
    transition: translate var(--motion-base) var(--ease-spring);
  }

  :global([dir="rtl"]) .thumb {
    translate: calc(var(--cell) * var(--index) * -1) 0;
  }

  .mode-switch[data-moved="true"] .thumb {
    animation: thumb-travel var(--motion-base) var(--ease-out);
  }

  @keyframes thumb-travel {
    40% {
      scale: 1.22 0.94;
    }
  }

  .mode {
    position: relative;
    display: grid;
    place-items: center;
    inline-size: var(--cell);
    block-size: var(--cell);
    padding: 0;
    border: 0;
    border-radius: var(--radius-capsule);
    background: transparent;
    color: var(--color-muted);
    cursor: default;
    transition:
      color var(--motion-fast) var(--ease-out),
      scale var(--motion-slow) var(--ease-spring);
  }

  .mode:hover {
    color: var(--color-text);
  }

  .mode:active {
    scale: 0.9;
    transition-duration: var(--motion-instant);
  }

  .mode[aria-pressed="true"] {
    color: var(--color-text);
  }

  .mode:focus-visible {
    outline: 2px solid var(--color-ring);
    outline-offset: -2px;
  }

  /* Beside a tool panel the rail has no header to hold it off the window's
     edge or to end it, so it carries that space itself. */
  .mode-switch[data-standalone="true"] {
    align-self: center;
    margin-block: 8px 14px;
  }

  @media (prefers-reduced-motion: reduce) {
    .thumb {
      transition: none;
    }

    .mode-switch[data-moved="true"] .thumb {
      animation: none;
    }
  }
</style>
