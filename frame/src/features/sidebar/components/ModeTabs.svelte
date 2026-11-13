<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import type * as MailDomain from "$domain/mail";
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
  // Mail's unread count rides on its segment. The control has no slot for it,
  // so the count is laid over the segment's corner and read out beside it.
  const mailAt = $derived(options.findIndex((option) => option.value === "mail"));
  // The mail domain is large and most sessions never open Mail, so it loads on
  // first need; the switch shows no count until it has.
  let mail = $state.raw<typeof MailDomain | null>(null);
  const unread = $derived(mail ? mail.mailUnread.total : 0);
  const mailBadge = $derived(mail ? mail.badgeText(mail.mailUnread.total) : null);
  // The count is held only while a Mail mode is on the switch.
  $effect(() => {
    if (mailAt < 0) return;
    let alive = true;
    let release: (() => void) | undefined;
    void import("$domain/mail").then((module) => {
      if (!alive) return;
      mail = module;
      release = module.mailUnread.hold();
    });
    return () => {
      alive = false;
      release?.();
    };
  });
</script>

<!--
  The modes, one choice: the shared control, sized to the column. The binding
  reads the surface native has confirmed and turns a pick into a request, so
  the thumb only travels once the mode has actually changed and can never be
  left on one native refused. With fewer than two modes on there is nothing
  to choose and no switch.
-->
{#if options.length < 2}{:else if compact}<ModeWord
    {standalone}
    count={unread}
    badge={mailBadge}
  />{:else}<div class="modes">
    <SegmentedControl
      label={m.ui_mode_work_hint()}
      {options}
      full
      size="regular"
      bind:value={() => current, (next) => chooseMode(next)}
    />
    {#if mailAt >= 0 && mailBadge !== null}
      <span class="badge" aria-hidden="true" style:--at={(mailAt + 1) / options.length}
        >{mailBadge}</span
      >
      <span class="sr-only">{m.mode_mail_unread({ count: unread })}</span>
    {/if}
  </div>{/if}

<style>
  .modes {
    position: relative;
    display: flex;
    flex: 1;
    min-width: 0;
  }

  /* In the column the switch is one more band of it, so it takes the corner
     every band there shares rather than a free-standing control's. */
  .modes :global(.segmented[data-size="regular"]) {
    --seg-radius: var(--radius-row);
  }

  /* The switch's padding is 3px; the segments share the rest equally, so the
     Mail segment's far edge sits at this fraction of the padded track. */
  .badge {
    position: absolute;
    z-index: 2;
    inset-block-start: 0;
    inset-inline-start: calc(3px + (100% - 6px) * var(--at));
    min-inline-size: 15px;
    height: 15px;
    box-sizing: border-box;
    padding-inline: 4px;
    border-radius: var(--radius-capsule);
    background: var(--color-accent);
    color: var(--color-on-accent);
    font-size: 9px;
    font-weight: 600;
    line-height: 15px;
    text-align: center;
    font-variant-numeric: tabular-nums;
    translate: -70% -35%;
    pointer-events: none;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    border: 0;
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: no-preference) {
    .badge {
      animation: badge-in var(--motion-fast) var(--ease-spring);
    }
  }

  @keyframes badge-in {
    from {
      scale: 0.5;
      opacity: 0;
    }
  }
</style>
