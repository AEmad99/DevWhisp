<script lang="ts">
  export let name: 'dashboard' | 'history' | 'settings';
  export let size: number = 18;
</script>

<svg
  viewBox="0 0 24 24"
  width={size}
  height={size}
  fill="none"
  stroke="currentColor"
  stroke-width="1.75"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === 'dashboard'}
    <rect x="3" y="3" width="7" height="9" rx="1.5" />
    <rect x="14" y="3" width="7" height="5" rx="1.5" />
    <rect x="14" y="11" width="7" height="10" rx="1.5" />
    <rect x="3" y="15" width="7" height="6" rx="1.5" />
  {:else if name === 'history'}
    <path d="M12 8v4l3 2" />
    <circle cx="12" cy="12" r="9" />
    <path d="M12 3v2" />
  {:else}
    <circle cx="12" cy="12" r="3" />
    <path
      d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41"
    />
  {/if}
</svg>