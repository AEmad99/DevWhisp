/**
 * Sanity-check the NSIS installer artifact after `npm run tauri:build`.
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');
const BUNDLE_DIR = path.join(ROOT, 'src-tauri', 'target', 'release', 'bundle', 'nsis');
const NSIS_SCRIPT = path.join(ROOT, 'src-tauri', 'target', 'release', 'nsis', 'x64', 'installer.nsi');
const MODEL_FILE = path.join(ROOT, 'src-tauri', 'resources', 'whisper-tiny-en', 'ggml-tiny.en.bin'); // optional now (runtime download)

function findInstaller() {
  if (!fs.existsSync(BUNDLE_DIR)) {
    throw new Error(`Bundle directory not found: ${BUNDLE_DIR}`);
  }
  const installers = fs
    .readdirSync(BUNDLE_DIR)
    .filter((name) => name.endsWith('-setup.exe'))
    .map((name) => path.join(BUNDLE_DIR, name));
  if (installers.length === 0) {
    throw new Error(`No NSIS setup.exe found in ${BUNDLE_DIR}`);
  }
  return installers[0];
}

function main() {
  const installer = findInstaller();
  const installerStat = fs.statSync(installer);
  const nsisScript = fs.readFileSync(NSIS_SCRIPT, 'utf8');

  // Model is no longer bundled (downloaded at runtime inside the app).
  // We verify it's NOT packed.
  const modelSourceExists = fs.existsSync(MODEL_FILE);
  const modelPacked = nsisScript.includes('ggml-tiny.en.bin');

  const checks = [
    {
      name: 'installer artifact',
      ok: installerStat.isFile() && installerStat.size >= 50_000_000,  // smaller now without bundled model
      detail: `${path.basename(installer)} (${(installerStat.size / 1_000_000).toFixed(1)} MB)`,
    },
    {
      name: 'model not bundled (runtime download)',
      ok: !modelPacked,
      detail: modelPacked ? 'unexpectedly packs model' : 'good - model downloaded inside app after install',
    },
    {
      name: 'NSIS bundles offline WebView2',
      ok: nsisScript.includes('INSTALLWEBVIEW2MODE "offlineInstaller"'),
      detail: 'offline WebView2 runtime installer (no internet required)',
    },
    {
      name: 'NSIS per-user install',
      ok: /INSTALLMODE.*currentUser/i.test(nsisScript) || nsisScript.includes('SetShellVarContext current'),
      detail: 'installs for current user without admin prompt',
    },
  ];

  let failed = false;
  console.log(`Installer: ${installer}`);
  for (const check of checks) {
    const status = check.ok ? 'OK' : 'FAIL';
    console.log(`[${status}] ${check.name} — ${check.detail}`);
    if (!check.ok) failed = true;
  }

  // Optional note about model source (if present)
  if (modelSourceExists) {
    console.log('[INFO] model source still exists in resources/ (used only for optional bundling builds)');
  }

  if (failed) {
    process.exit(1);
  }
  console.log('Installer verification passed (self-contained, runtime model download).');
}

main();