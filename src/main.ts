import { mount, unmount } from 'svelte'
import './app.css'
import App from './App.svelte'

// Apply persisted UI preferences (accent + font scale) before mount so the
// first paint already reflects the user's chosen look — no flash of default
// theme. localStorage keys mirror Settings.svelte's LS_KEYS.
function applyUiPrefs() {
  try {
    const accent = window.localStorage.getItem('devwhisp.settings.accent')
    if (accent) {
      document.documentElement.setAttribute('data-accent', accent)
    }
    const fontScale = window.localStorage.getItem('devwhisp.settings.fontScale')
    if (fontScale) {
      const n = Number(fontScale)
      if (Number.isFinite(n)) {
        document.documentElement.style.setProperty(
          '--font-scale',
          String(Math.max(0.8, Math.min(1.25, n / 100))),
        )
      }
    }
  } catch {
    /* localStorage unavailable; fall back to defaults */
  }
}
applyUiPrefs()

// Global error reporters. Surface any uncaught failure directly in the
// splash element so the user (and we) can see what went wrong instead of
// staring at a stuck "Loading…" forever.
function showFatal(message: string) {
  const splash = document.getElementById('app-splash')
  if (!splash) return
  splash.innerHTML =
    `<div style="max-width:520px;padding:24px 32px;color:#ff5c7c;font-family:ui-monospace,monospace;font-size:13px;line-height:1.5;text-align:left;">` +
    `<strong style="color:#f3f0fb;display:block;margin-bottom:8px;">DevWhisp failed to start</strong>` +
    `<span>${message.replace(/[<>&]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;' }[c] ?? c))}</span></div>`
  splash.style.alignItems = 'flex-start'
  splash.style.justifyContent = 'flex-start'
}

window.addEventListener('error', (e) => {
  showFatal(`Uncaught error: ${e.message ?? String(e.error ?? e)}`)
})
window.addEventListener('unhandledrejection', (e) => {
  const reason = e.reason instanceof Error ? e.reason.message : String(e.reason)
  showFatal(`Unhandled rejection: ${reason}`)
})

function hideSplash() {
  const splash = document.getElementById('app-splash')
  if (!splash) return
  splash.classList.add('hide')
  // Remove from DOM after the fade so it cannot interfere with focus / events.
  setTimeout(() => splash.remove(), 220)
}

let app: ReturnType<typeof mount> | null = null
try {
  app = mount(App, {
    target: document.getElementById('app')!,
    // intro: false keeps the first frame from animating in — we just want
    // a clean swap from splash to rendered app.
    intro: false,
  })

  // Hide the splash on the next animation frame so the Svelte render has
  // had a chance to paint. Two rAFs is the standard "wait until paint"
  // pattern and avoids the user seeing the splash flash for one frame
  // behind the new content.
  requestAnimationFrame(() => {
    requestAnimationFrame(hideSplash)
  })
} catch (e) {
  const msg = e instanceof Error ? `${e.message}\n${e.stack ?? ''}` : String(e)
  showFatal(`Mount failed: ${msg}`)
  throw e
}

// Hot-module-replacement teardown (vite dev only). In production this
// function is never called because the entry script isn't re-evaluated.
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    if (app) unmount(app)
  })
}

export default app


