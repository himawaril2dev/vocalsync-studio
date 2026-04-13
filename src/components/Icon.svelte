<script lang="ts">
  interface Props {
    name: string;
    size?: number;
  }

  let { name, size = 16 }: Props = $props();
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === "play"}
    <polygon points="6,3 20,12 6,21" fill="currentColor" stroke="none" />
  {:else if name === "pause"}
    <rect x="5" y="4" width="4" height="16" fill="currentColor" stroke="none" />
    <rect x="15" y="4" width="4" height="16" fill="currentColor" stroke="none" />
  {:else if name === "stop"}
    <rect x="4" y="4" width="16" height="16" rx="2" fill="currentColor" stroke="none" />
  {:else if name === "record"}
    <circle cx="12" cy="12" r="8" fill="currentColor" stroke="none" />
  {:else if name === "download"}
    <path d="M12 3v12m0 0l-4-4m4 4l4-4" />
    <path d="M4 17v2a2 2 0 002 2h12a2 2 0 002-2v-2" />
  {:else if name === "export"}
    <path d="M12 3v12m0 0l-4-4m4 4l4-4" />
    <path d="M4 17v2a2 2 0 002 2h12a2 2 0 002-2v-2" />
  {/if}
</svg>

<style>
  .icon {
    display: inline-block;
    vertical-align: middle;
    flex-shrink: 0;
  }
</style>
