<script lang="ts">
  import Icon from "$shared/ui/Icon";
  import FavIcon from "$shared/ui/FavIcon";
  import { Calendar01Icon, Globe02Icon } from "@hugeicons/core-free-icons";
  import { parseEventUrl } from "$domain/calendar-link";
  import { favicons } from "$domain/favicons";

  let { url, size = 12 }: { url: string; size?: number } = $props();
  let mark = $derived(favicons.forPage(url));
</script>

{#if parseEventUrl(url)}<Icon icon={Calendar01Icon} {size} />{:else if mark}<FavIcon
    image={mark.image}
    tone={mark.tone}
    {size}
    lit
    fallback={Globe02Icon}
  />{:else}<Icon icon={Globe02Icon} {size} />{/if}
