/**
 * Ensures the bundled whisper-tiny.en model is present before `tauri build`.
 * Downloads from Hugging Face when missing or incomplete (~75 MB).
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { pipeline } from 'node:stream/promises';
import { Readable } from 'node:stream';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');
const MODEL_DIR = path.join(ROOT, 'src-tauri', 'resources', 'whisper-tiny-en');
const MODEL_FILE = path.join(MODEL_DIR, 'ggml-tiny.en.bin');
const MODEL_URL =
  'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin';
/** Reject truncated copies; the published file is ~77 MB. */
const MIN_BYTES = 70_000_000;

function isModelReady() {
  try {
    const stat = fs.statSync(MODEL_FILE);
    return stat.isFile() && stat.size >= MIN_BYTES;
  } catch {
    return false;
  }
}

async function downloadModel() {
  fs.mkdirSync(MODEL_DIR, { recursive: true });
  const tmp = `${MODEL_FILE}.download`;
  console.log(`Downloading whisper-tiny.en model to ${MODEL_FILE} ...`);

  const res = await fetch(MODEL_URL, {
    headers: { 'User-Agent': 'DevWhisp-build/0.1' },
    redirect: 'follow',
  });
  if (!res.ok) {
    throw new Error(`Model download failed: HTTP ${res.status} ${res.statusText}`);
  }
  if (!res.body) {
    throw new Error('Model download failed: empty response body');
  }

  await pipeline(Readable.fromWeb(res.body), fs.createWriteStream(tmp));
  const size = fs.statSync(tmp).size;
  if (size < MIN_BYTES) {
    fs.unlinkSync(tmp);
    throw new Error(`Downloaded model looks truncated (${size} bytes)`);
  }
  fs.renameSync(tmp, MODEL_FILE);
  console.log(`Model ready (${(size / 1_000_000).toFixed(1)} MB).`);
}

async function main() {
  if (isModelReady()) {
    const size = fs.statSync(MODEL_FILE).size;
    console.log(`Bundled model present (${(size / 1_000_000).toFixed(1)} MB).`);
    return;
  }
  await downloadModel();
}

main().catch((err) => {
  console.error(err instanceof Error ? err.message : err);
  console.error(
    '\nThe offline installer bundles this model. Fetch it manually or rerun:\n' +
      '  npm run fetch:model',
  );
  process.exit(1);
});