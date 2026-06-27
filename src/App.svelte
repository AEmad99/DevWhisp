<script lang="ts">
  /**
   * App shell — sidebar navigation + view switcher.
   *
   * Each view lives in src/lib/{Dashboard,History,Settings}.svelte.
   * App.svelte is intentionally thin: state for which view is active,
   * the sidebar UI, the keyboard shortcuts, and a global IPC-error toast.
   */

	  import { onMount } from 'svelte';
	  import { listen } from '@tauri-apps/api/event';
	  import Dashboard from './lib/Dashboard.svelte';
	  import History from './lib/History.svelte';
	  import Settings from './lib/Settings.svelte';
	  import AppIcon from './lib/AppIcon.svelte';
	  import NavIcon from './lib/NavIcon.svelte';
  import OnboardingWizard from './lib/OnboardingWizard.svelte';
	  import { getAppInfo, type AppInfo, type IpcError, formatIpcError } from './lib/api';

  type View = 'dashboard' | 'history' | 'settings';

  let currentView = $state<View>('dashboard');
  let appInfo = $state<AppInfo | null>(null);

  // First-run onboarding — read synchronously so the wizard doesn't flash.
  const ONBOARD_KEY = 'devwhisp.onboarded';
  function readOnboarded(): boolean {
    try {
      return window.localStorage.getItem(ONBOARD_KEY) === '1';
    } catch {
      return true;
    }
  }
  let onboarded = $state(readOnboarded());
  function finishOnboarding() {
    onboarded = true;
    try {
      window.localStorage.setItem(ONBOARD_KEY, '1');
    } catch {
      /* best-effort */
    }
  }

  /** Tiny global toast queue for IPC errors. One at a time, 4s timeout. */
  let toast = $state<{ id: number; message: string } | null>(null);

  function showToast(message: string) {
    const id = Date.now() + Math.random();
    toast = { id, message };
    window.setTimeout(() => {
      if (toast && toast.id === id) toast = null;
    }, 4000);
  }

  /** Surface any uncaught IPC failure to the toast layer. */
  function handleGlobalError(err: unknown) {
    if (err && typeof err === 'object' && 'kind' in err) {
      showToast(formatIpcError(err as IpcError));
    } else if (err instanceof Error) {
      showToast(err.message);
    } else if (typeof err === 'string') {
      showToast(err);
    }
  }

  // Catch promise rejections from components that don't handle their own errors.
  // Note: components already handle most IPC errors inline; this is a safety net
  // for things like the Settings dictionary or History load failing async.
  onMount(() => {
    const onRejection = (event: PromiseRejectionEvent) => {
      handleGlobalError(event.reason);
    };
    window.addEventListener('unhandledrejection', onRejection);
    window.addEventListener('error', (event) => {
      if (event.error) handleGlobalError(event.error);
    });

    getAppInfo()
      .then((info) => (appInfo = info))
      .catch(() => {
        /* non-fatal; sidebar shows fallback */
      });

    // Tray "Open History" / "Settings…" emit a `navigate` event; route it to
    // the matching view. The tray reveals the window before emitting.
    // Wrapped defensively: outside a Tauri runtime (e.g. a plain browser),
    // `listen` throws synchronously — navigation events are optional, so a
    // missing bridge must never take down the whole shell.
    let unlistenNav: (() => void) | null = null;
    try {
      listen<string>('navigate', (event) => {
        const view = event.payload;
        if (view === 'dashboard' || view === 'history' || view === 'settings') {
          currentView = view;
        }
      })
        .then((un) => {
          unlistenNav = un;
        })
        .catch(() => {});
    } catch {
      /* no Tauri bridge — fine */
    }

    return () => {
      window.removeEventListener('unhandledrejection', onRejection);
      if (unlistenNav) unlistenNav();
    };
  });

  // ---- Keyboard shortcuts ---------------------------------------------
  // Ctrl+, → Settings, Ctrl+H → History. Don't shadow browser devtools.
  // We attach to document so the shortcut works regardless of focus, but
  // we skip when the user is typing in an input/textarea/contenteditable.
  function onShortcutKeydown(event: KeyboardEvent) {
    if (!(event.ctrlKey || event.metaKey)) return;
    // Skip if the user is typing in a text field.
    const target = event.target as HTMLElement | null;
    if (target) {
      const tag = target.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || target.isContentEditable) {
        return;
      }
    }
    if (event.altKey || event.shiftKey) return;

    switch (event.key.toLowerCase()) {
      case ',':
        event.preventDefault();
        currentView = 'settings';
        break;
      case 'h':
        // Don't shadow Ctrl+Shift+I/J/C (devtools). Ctrl+H alone is fine.
        event.preventDefault();
        currentView = 'history';
        break;
      case 'd':
        // Ctrl+D = bookmark; skip. But offer a dashboard shortcut for symmetry.
        event.preventDefault();
        currentView = 'dashboard';
        break;
    }
  }

  onMount(() => {
    document.addEventListener('keydown', onShortcutKeydown);
    return () => {
      document.removeEventListener('keydown', onShortcutKeydown);
    };
  });

  const navItems: { id: View; label: string; hint: string }[] = [
    { id: 'dashboard', label: 'Home', hint: 'Ctrl+D' },
    { id: 'history', label: 'History', hint: 'Ctrl+H' },
    { id: 'settings', label: 'Settings', hint: 'Ctrl+,' },
  ];
</script>

<div class="shell">
  <aside class="dock" aria-label="Primary navigation">
    <div class="dock-brand">
      <span class="dock-icon" aria-hidden="true">
        <AppIcon size={32} />
      </span>
      <span class="dock-name">DevWhisp</span>
    </div>

    <nav class="dock-nav">
      {#each navItems as item (item.id)}
        <button
          type="button"
          class="dock-item"
          class:active={currentView === item.id}
          aria-current={currentView === item.id ? 'page' : undefined}
          title="{item.label} ({item.hint})"
          onclick={() => (currentView = item.id)}
        >
          <span class="dock-item-icon" aria-hidden="true">
            <NavIcon name={item.id} />
          </span>
          <span class="dock-item-label">{item.label}</span>
        </button>
      {/each}
    </nav>

    <div class="dock-footer">
      <span class="dock-version">v{appInfo?.version ?? '0.1.0'}</span>
    </div>
  </aside>

  <main class="content">
    <div class="content-inner">
      {#if currentView === 'dashboard'}
        <Dashboard />
      {:else if currentView === 'history'}
        <History />
      {:else if currentView === 'settings'}
        <Settings />
      {/if}
    </div>
  </main>

  {#if toast}
    <div class="toast" role="status" aria-live="polite">
      <span class="toast-dot"></span>
      <span class="toast-msg">{toast.message}</span>
      <button
        class="toast-close"
        aria-label="Dismiss"
        onclick={() => (toast = null)}
      >×</button>
    </div>
  {/if}

  {#if !onboarded}
    <OnboardingWizard done={finishOnboarding} />
  {/if}
</div>

<style>
  /* Layout shell — side dock + content. */
  .shell {
    display: grid;
    grid-template-columns: 200px 1fr;
    min-height: 100vh;
  }

  /* Side dock — always vertical, never collapses to top nav. */
  .dock {
    background: var(--bg-elevated);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    padding: 20px 12px 16px;
    gap: 24px;
    position: sticky;
    top: 0;
    align-self: start;
    height: 100vh;
    min-width: 200px;
  }

  .dock-brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 8px 8px;
  }
  .dock-icon {
    display: inline-flex;
    flex-shrink: 0;
    filter: drop-shadow(0 2px 8px rgba(124, 58, 237, 0.35));
  }
  .dock-name {
    font-weight: 600;
    font-size: 15px;
    letter-spacing: -0.02em;
    color: var(--text);
  }

  .dock-nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    padding: 0 4px;
  }

  .dock-item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 10px 12px;
    background: transparent;
    border: none;
    border-radius: 10px;
    color: var(--muted);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    transition: background 140ms ease, color 140ms ease;
  }
  .dock-item:hover {
    color: var(--text);
    background: rgba(124, 58, 237, 0.08);
  }
  .dock-item.active {
    color: var(--text);
    background: rgba(124, 58, 237, 0.14);
  }
  .dock-item.active::before {
    content: '';
    position: absolute;
    left: 0;
    top: 8px;
    bottom: 8px;
    width: 3px;
    background: var(--accent-deep);
    border-radius: 0 3px 3px 0;
  }
  .dock-item-icon {
    display: inline-flex;
    flex-shrink: 0;
    opacity: 0.85;
  }
  .dock-item.active .dock-item-icon {
    color: var(--accent);
    opacity: 1;
  }
  .dock-item-label {
    flex: 1;
    min-width: 0;
  }

  .dock-footer {
    padding: 12px 12px 0;
    border-top: 1px solid var(--border);
  }
  .dock-version {
    color: var(--muted);
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: 11px;
    opacity: 0.65;
  }

  /* Content */
  .content {
    min-width: 0;
    overflow-y: auto;
    height: 100vh;
  }
  .content-inner {
    max-width: 720px;
    margin: 0 auto;
    padding: 28px 36px 80px;
  }

  /* Toast */
  .toast {
    position: fixed;
    bottom: 18px;
    right: 18px;
    z-index: 50;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: rgba(20, 16, 30, 0.95);
    border: 1px solid var(--danger);
    border-left-width: 3px;
    border-radius: 10px;
    color: var(--text);
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: 12px;
    max-width: 360px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
    animation: toast-in 220ms ease-out;
  }
  .toast-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--danger);
    flex-shrink: 0;
  }
  .toast-msg {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .toast-close {
    background: transparent;
    border: none;
    color: var(--muted);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
    padding: 0 4px;
  }
  .toast-close:hover { color: var(--text); }

  @keyframes toast-in {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

</style>