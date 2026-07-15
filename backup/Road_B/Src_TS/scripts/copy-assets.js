// copy-assets.js — copy renderer static files (HTML/CSS) into dist/.
//
// tsc only emits .js; the renderer's index.html and style.css must be placed
// next to the compiled renderer.js so Electron's loadFile() finds them.

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const srcDir = path.join(root, "src", "renderer");
const outDir = path.join(root, "dist", "renderer");

fs.mkdirSync(outDir, { recursive: true });

for (const f of ["index.html", "style.css"]) {
  const from = path.join(srcDir, f);
  const to = path.join(outDir, f);
  fs.copyFileSync(from, to);
  console.log(`copied ${f} -> dist/renderer/${f}`);
}
