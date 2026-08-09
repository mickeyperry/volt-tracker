#!/usr/bin/env node
/* Stage the app for the desktop shell.
   VOLT is one self-contained file, so "staging" is a copy and nothing else — no bundler, no
   transform, no chance of the desktop build drifting from the web one. It exists because Tauri
   bundles a DIRECTORY, and pointing it at the repo root made it try to read its own target/
   folder while cargo held it open.
   Run automatically by `cargo tauri dev` and `cargo tauri build` (see beforeDevCommand). */
const fs = require('fs');
const path = require('path');

const ROOT = path.join(__dirname, '..');
const SRC = process.env.VOLT_FILE || 'beta2.html';
const OUT = path.join(ROOT, 'native', 'desktop', 'dist');

const src = path.join(ROOT, SRC);
if (!fs.existsSync(src)) {
  console.error('mkdesktop: no such file: ' + SRC);
  process.exit(1);
}
fs.mkdirSync(OUT, { recursive: true });
fs.copyFileSync(src, path.join(OUT, 'index.html'));
const kb = (fs.statSync(src).size / 1024).toFixed(0);
console.log('mkdesktop: ' + SRC + ' -> native/desktop/dist/index.html (' + kb + ' KB)');
