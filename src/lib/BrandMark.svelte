<script lang="ts">
  /**
   * Compact Voice-Code mark for the pill — angle brackets < > framing three
   * horizontal smoky bars, no tile. Precision solid styling.
   */
  export let size: number = 28;
  export let gradient: boolean = true;
  export let color: string = 'var(--accent)';
</script>

<svg
  viewBox="0 0 32 32"
  width={size}
  height={size}
  aria-hidden="true"
  overflow="visible"
>
  <!-- Brackets -->
  <g fill="none" stroke={gradient ? color : '#ffffff'} stroke-linecap="round" stroke-linejoin="round">
    <path d="M 13.2 7 L 8.2 16 L 13.2 25" stroke-width="2.4" />
    <path d="M 18.8 7 L 23.8 16 L 18.8 25" stroke-width="2.4" />
  </g>

  <!-- Three horizontal smoky bars -->
  <g fill={gradient ? color : '#ffffff'}>
    <path d="M 12.3 11.2 C 12.3 10.3 12.7 9.9 13.5 10.0 L 18.5 10.0 C 19.3 9.9 19.7 10.3 19.7 11.2 C 19.7 12.1 19.3 12.5 18.5 12.4 L 13.5 12.4 C 12.7 12.5 12.3 12.1 12.3 11.2 Z" fill-opacity="0.55" />
    <path d="M 11.4 15.6 C 11.4 14.6 11.9 14.1 12.8 14.2 L 19.2 14.2 C 20.1 14.1 20.6 14.6 20.6 15.6 C 20.6 16.6 20.1 17.1 19.2 17.0 L 12.8 17.0 C 11.9 17.1 11.4 16.6 11.4 15.6 Z" fill-opacity="0.80" />
    <path d="M 12.3 20.2 C 12.3 19.2 12.8 18.7 13.7 18.8 L 18.3 18.8 C 19.2 18.7 19.7 19.2 19.7 20.2 C 19.7 21.2 19.2 21.7 18.3 21.6 L 13.7 21.6 C 12.8 21.7 12.3 21.2 12.3 20.2 Z" fill-opacity="1" />
  </g>
</svg>
