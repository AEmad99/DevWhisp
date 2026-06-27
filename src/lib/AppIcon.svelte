<script lang="ts">
  /**
   * DevWhisp brand mark — matches the application icon exactly ("Voice Waves"):
   *   - violet rounded-square tile (family-matched: inset 96, radius 196,
   *     gradient #c4b5fd → #7c3aed) shared with DevTerm and DevSpace
   *   - one solid white source dot + three concentric white voice-wave arcs
   *     radiating right (opacity tapers to whisper outward)
   *
   * Sized via the `size` prop. Crisp at any size (pure vector). Used by the
   * sidebar brand and the Dashboard hero.
   */
  export let size: number = 32;
</script>

<svg viewBox="0 0 1024 1024" width={size} height={size} aria-hidden="true">
  <defs>
    <linearGradient id="devwhispIconBg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#c4b5fd" />
      <stop offset="100%" stop-color="#7c3aed" />
    </linearGradient>
  </defs>

  <!-- Family-matched container -->
  <rect x="96" y="96" width="832" height="832" rx="196" ry="196" fill="url(#devwhispIconBg)" />
  <rect x="99" y="99" width="826" height="826" rx="193" fill="none" stroke="#ffffff" stroke-opacity="0.18" stroke-width="6" />

  <!-- Three concentric voice-wave arcs radiating right from the source dot. -->
  <g fill="none" stroke="#ffffff" stroke-linecap="round" stroke-linejoin="round">
    <path d="M 415.34 389.03 A 152 152 0 0 1 415.34 634.97" stroke-width="66" stroke-opacity="1" />
    <path d="M 491.76 283.86 A 282 282 0 0 1 491.76 740.14" stroke-width="58" stroke-opacity="0.8" />
    <path d="M 568.17 178.68 A 412 412 0 0 1 568.17 845.32" stroke-width="50" stroke-opacity="0.6" />
  </g>

  <!-- The voice source: solid white dot -->
  <circle cx="326" cy="512" r="62" fill="#ffffff" />
</svg>
