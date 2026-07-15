/*
 * copy-assets.js — copy non-TS renderer assets (index.html) into dist/.
 *
 * tsc only emits .js; the Electron main process loads dist/renderer/index.html,
 * so we mirror src/renderer/index.html there after compilation.
 */
'use strict';

const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..');
const srcHtml = path.join(root, 'src', 'renderer', 'index.html');
const outDir = path.join(root, 'dist', 'renderer');
const outHtml = path.join(outDir, 'index.html');

fs.mkdirSync(outDir, { recursive: true });
fs.copyFileSync(srcHtml, outHtml);
process.stdout.write(`copied ${srcHtml} -> ${outHtml}\n`);
