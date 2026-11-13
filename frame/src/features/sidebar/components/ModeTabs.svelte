<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import { surface } from "$domain/surface";
  import SegmentedControl from "$shared/ui/SegmentedControl";
  import { chooseMode, modeOptions } from "../lib/modes";
  import ModeWord from "./ModeWord.svelte";

  let {
    compact = false,
    standalone = false,
  }: {
    /** The rail's width: the environment's name, opening onto the other. */
    compact?: boolean;
    /** Heads a rail beside a tool panel, with no header under it to end it. */
    standalone?: boolean;
  } = $props();

  const options = $derived(modeOptions());
  const current = $derived(surface.modeOf(surface.currentPage()));
</script>

<!--
  The modes, one choice: the shared control, sized to the column. The binding
  reads the surface native has confirmed and turns a pick into a request, so
  the thumb only travels once the mode has actually changed and can never be
  left on one native refused. With fewer than two modes on there is nothing
  to choose and no switch.
-->
{#if options.length < 2}{:else if compact}<ModeWord {standalone} />{:else}<div class="modes">
    <SegmentedControl
      label={m.ui_mode_work_hint()}
      {options}
      full
      size="regular"
      bind:value={() => current, (next) => chooseMode(next)}
    />
  </div>{/if}

<style>
  .modes {
    display: flex;
    flex: 1;
    min-width: 0;
  }

  /* In the column the switch is one more band of it, so it takes the corner
     every band there shares rather than a free-standing control's. */
  .modes :global(.segmented[data-size="regular"]) {
    --seg-radius: var(--radius-row);
  }
</style>
