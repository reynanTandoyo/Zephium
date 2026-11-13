<!--
  Mail as a full-window mode. A placeholder until the feature lands: the
  title and a note, centred in the page the mode gives the window.
-->
<script lang="ts">
  import { Mail01Icon } from "@hugeicons/core-free-icons";
  import * as m from "$shared/i18n/messages";
  import EmptyState from "$shared/ui/EmptyState";
  import Icon from "$shared/ui/Icon";
</script>

<section class="page" aria-label={m.mode_mail()}>
  <EmptyState title={m.mode_mail()} description={m.mode_coming_soon()}>
    {#snippet icon()}<Icon icon={Mail01Icon} size={28} />{/snippet}
  </EmptyState>
</section>

<style>
  .page {
    display: grid;
    block-size: 100%;
    place-items: center;
    color: var(--color-text);
  }
</style>
