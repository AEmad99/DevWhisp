import { mount, unmount } from 'svelte';
import '../app.css';
import Pill from './Pill.svelte';

// Apply the user's persisted accent + font scale to <html> before mount so
// the pill window's first paint matches the user's chosen look. Mirrors
// `applyUiPrefs()` in main.ts.
function applyUiPrefs() {
  try {
    const accent = window.localStorage.getItem('devwhisp.settings.accent');
    if (accent) {
      document.documentElement.setAttribute('data-accent', accent);
    }
    const fontScale = window.localStorage.getItem('devwhisp.settings.fontScale');
    if (fontScale) {
      const n = Number(fontScale);
      if (Number.isFinite(n)) {
        document.documentElement.style.setProperty(
          '--font-scale',
          String(Math.max(0.8, Math.min(1.25, n / 100))),
        );
      }
    }
  } catch {
    /* localStorage unavailable; fall back to defaults */
  }
}
applyUiPrefs();

function showFatal(message: string) {
  const target = document.getElementById('pill') ?? document.body;
  target.innerHTML =
    `<div style="padding:10px 14px;color:#ff5c7c;font-family:ui-monospace,monospace;font-size:11px;">` +
    `<strong style="color:#f3f0fb;display:block;margin-bottom:4px;">Pill failed to start</strong>` +
    `<span>${message.replace(/[<>&]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;' }[c] ?? c))}</span></div>`;
}

window.addEventListener('error', (e) => {
  showFatal(`Uncaught: ${e.message ?? String(e.error ?? e)}`);
});
window.addEventListener('unhandledrejection', (e) => {
  const reason = e.reason instanceof Error ? e.reason.message : String(e.reason);
  showFatal(`Unhandled: ${reason}`);
});

const target = document.getElementById('pill');
if (!target) {
  showFatal('pill mount target missing');
} else {
  let app: ReturnType<typeof mount> | null = null;
  try {
    app = mount(Pill, { target, intro: false });
  } catch (e) {
    const msg = e instanceof Error ? `${e.message}\n${e.stack ?? ''}` : String(e);
    showFatal(`Mount failed: ${msg}`);
    throw e;
  }
  if (import.meta.hot) {
    import.meta.hot.dispose(() => {
      if (app) unmount(app);
    });
  }
}

export default null;