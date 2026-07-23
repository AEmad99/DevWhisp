<script lang="ts">
  /**
   * First-run onboarding. Five quick steps: welcome → mic → model → hotkey →
   * try it. Shown once (gated by localStorage in App.svelte). Calls `done`
   * when finished or skipped.
   */
  import { onMount } from 'svelte';
  import AppIcon from './AppIcon.svelte';
  import { getModelStatus, getHotkey, type ModelStatus } from './api';

  let { done }: { done: () => void } = $props();

  let step = $state(0);
  let model = $state<ModelStatus | null>(null);
  let hotkeyLabel = $state('Ctrl+Shift+Space');
  let hotkeyKeys = $derived(hotkeyLabel.split('+').map((s) => s.trim()).filter(Boolean));

  const STEPS = [
    { key: 'welcome', label: 'Welcome' },
    { key: 'mic', label: 'Microphone' },
    { key: 'model', label: 'Model' },
    { key: 'hotkey', label: 'Hotkey' },
    { key: 'try', label: 'Try it' },
  ];
  const last = STEPS.length - 1;

  onMount(() => {
    getModelStatus().then((m) => (model = m)).catch(() => {});
    getHotkey().then((h) => (hotkeyLabel = h)).catch(() => {});
  });

  function next() {
    if (step < last) step += 1;
    else done();
  }
  function back() {
    if (step > 0) step -= 1;
  }
</script>

<div class="overlay" role="dialog" aria-modal="true" aria-label="DevWhisp setup">
  <div class="wizard">
    <button class="skip" onclick={done}>Skip</button>

    <div class="panel">
      {#if step === 0}
        <span class="hero-icon"><AppIcon size={64} /></span>
        <h2>Talk instead of type</h2>
        <p>
          DevWhisp turns your voice into text and pastes it wherever your cursor is —
          fully offline, running on your CPU. Let's get you set up in a few seconds.
        </p>
      {:else if step === 1}
        <span class="step-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="3" width="6" height="12" rx="3" />
            <path d="M5 11a7 7 0 0 0 14 0" />
            <line x1="12" y1="18" x2="12" y2="22" />
            <line x1="8" y1="22" x2="16" y2="22" />
          </svg>
        </span>
        <h2>Microphone access</h2>
        <p>
          DevWhisp listens only while you hold the hotkey. Windows may prompt for
          microphone permission the first time you record. Your audio never leaves
          this machine.
        </p>
      {:else if step === 2}
        <span class="step-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
          </svg>
        </span>
        <h2>On-device model</h2>
        <p>
          Download an on-device Whisper model (Base recommended — same default as BridgeVoice).
          It runs fully offline on your CPU. One-time download (~142 MB for Base).
        </p>
        <div class="status {model?.ready ? 'ready' : 'pending'}">
          {#if model?.ready}
            ● Model ready · {model.fileSizeMb} MB
          {:else if model}
            Preparing model… {model.fileSizeMb}/{model.expectedSizeMb} MB
          {:else}
            Checking model…
          {/if}
        </div>
      {:else if step === 3}
        <span class="step-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="4" width="20" height="16" rx="2" />
            <path d="M6 8h.01M6 12h.01M6 16h.01" />
          </svg>
        </span>
        <h2>Your hotkey</h2>
        <p>Hold this anywhere, speak, and release to paste:</p>
        <div class="keys">
          {#each hotkeyKeys as k, i (k + i)}<kbd>{k}</kbd>{#if i < hotkeyKeys.length - 1}<span>+</span>{/if}{/each}
        </div>
        <p class="hint">Want a different one? Rebind it from <strong>Settings → Recording</strong> any time.</p>
      {:else}
        <span class="step-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 2l2.4 7.2h7.6l-6 4.8 2.4 7.2-6-4.8-6 4.8 2.4-7.2-6-4.8h7.6z" />
          </svg>
        </span>
        <h2>Give it a go</h2>
        <p>
          Click into any text field, hold
          {#each hotkeyKeys as k, i (k + i)}<kbd>{k}</kbd>{#if i < hotkeyKeys.length - 1}<span>+</span>{/if}{/each},
          and say <em>"hello world"</em>. Your words appear right where the cursor is.
        </p>
        <p class="hint">The floating pill shows you when DevWhisp is listening.</p>
      {/if}
    </div>

    <div class="dots" aria-hidden="true">
      {#each STEPS as s, i (s.key)}
        <span class="dot" class:active={i === step} class:done={i < step}></span>
      {/each}
    </div>

    <div class="actions">
      <button class="ghost" onclick={back} disabled={step === 0}>Back</button>
      <button class="primary" onclick={next}>{step === last ? 'Done' : 'Next'}</button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(8, 6, 14, 0.7);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    animation: fade 200ms ease-out;
  }
  @keyframes fade { from { opacity: 0; } to { opacity: 1; } }

  .wizard {
    position: relative;
    width: 100%;
    max-width: 400px;
    background: var(--card);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-lg);
    padding: 28px 24px 18px;
    box-shadow: var(--shadow-3);
    text-align: center;
  }

  .skip {
    position: absolute;
    top: 10px;
    right: 12px;
    background: transparent;
    border: none;
    color: var(--muted);
    font-size: 11px;
    cursor: pointer;
    font-family: inherit;
  }
  .skip:hover { color: var(--text); }

  .panel { min-height: 190px; display: flex; flex-direction: column; align-items: center; justify-content: center; }
  .hero-icon { margin-bottom: 12px; }
  .step-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    color: var(--accent);
    margin-bottom: 10px;
  }
  .step-icon svg { width: 100%; height: 100%; }

  h2 { margin: 0 0 8px; font-size: 18px; font-weight: 700; letter-spacing: -0.02em; color: var(--text); }
  p { margin: 0 0 8px; font-size: 13px; line-height: 1.5; color: var(--muted); max-width: 320px; }
  p strong, p em { color: var(--text); font-style: normal; }
  .hint { font-size: 12px; color: var(--faint); }

  .status {
    margin-top: 8px;
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: 12px;
    padding: 6px 12px;
    border-radius: 999px;
    border: 1px solid var(--border);
  }
  .status.ready { color: var(--ok); border-color: rgba(52, 211, 153, 0.3); }
  .status.pending { color: var(--warn); }

  .keys { display: flex; align-items: center; gap: 6px; margin: 6px 0 12px; }
  .keys span { color: var(--muted); }
  kbd {
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    border-radius: 6px;
    padding: 4px 9px;
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: 12px;
    color: var(--text);
  }

  .dots { display: flex; justify-content: center; gap: 7px; margin: 18px 0; }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--border-strong); transition: all 200ms ease; }
  .dot.active { background: var(--accent); width: 22px; border-radius: 999px; }
  .dot.done { background: var(--accent-deep); }

  .actions { display: flex; gap: 10px; }
  .actions button {
    flex: 1;
    padding: 11px 18px;
    border-radius: var(--r-md);
    font-size: 14px;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition: all 150ms ease;
  }
  .actions .ghost { background: transparent; border: 1px solid var(--border); color: var(--text); }
  .actions .ghost:hover:not(:disabled) { border-color: var(--accent); }
  .actions .ghost:disabled { opacity: 0.4; cursor: not-allowed; }
  .actions .primary { background: var(--accent-deep); border: none; color: #fff; box-shadow: var(--shadow-1); }
  .actions .primary:hover { filter: brightness(1.1); }
</style>
