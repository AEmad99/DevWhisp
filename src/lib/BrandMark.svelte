<script lang="ts">
  /** Compact voice-waves brand mark — matches the canonical app icon glyph. */
  export let size: number = 28;
  export let gradient: boolean = true;
</script>

<svg viewBox="0 0 64 64" width={size} height={size} aria-hidden="true">
  {#if gradient}
    <defs>
      <linearGradient id="brandGrad" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0%" stop-color="#c4b5fd" />
        <stop offset="100%" stop-color="#7c3aed" />
      </linearGradient>
    </defs>
  {/if}

  <g fill="none"
     stroke={gradient ? 'url(#brandGrad)' : '#ffffff'}
     stroke-linecap="round"
     stroke-linejoin="round">
    <path d="M 25.96 24.31 A 9.5 9.5 0 0 1 25.96 39.69" stroke-width="4.125" />
    <path d="M 30.74 17.74 A 17.625 17.625 0 0 1 30.74 46.26" stroke-width="3.625" stroke-opacity="0.85" />
    <path d="M 35.51 11.17 A 25.75 25.75 0 0 1 35.51 52.83" stroke-width="3.125" stroke-opacity="0.65" />
  </g>
  <circle
    cx="20.375"
    cy="32"
    r="3.875"
    fill={gradient ? 'url(#brandGrad)' : '#ffffff'}
  />
</svg>
