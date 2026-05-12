import { mkdir, readFile } from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";

const width = 660;
const height = 400;
const outputPath = path.join("src-tauri", "icons", "dmg-background.png");

const packageJson = JSON.parse(await readFile("package.json", "utf8"));
const version = packageJson.version ? `v${packageJson.version}` : "";

const escapeXml = (value) =>
  String(value).replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      default:
        return "&apos;";
    }
  });

const svg = `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <defs>
    <radialGradient id="glow" cx="28%" cy="42%" r="62%">
      <stop offset="0%" stop-color="#00d4aa" stop-opacity="0.18"/>
      <stop offset="42%" stop-color="#00d4aa" stop-opacity="0.055"/>
      <stop offset="100%" stop-color="#00d4aa" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="arrow" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#526170" stop-opacity="0.25"/>
      <stop offset="48%" stop-color="#8ba0ad" stop-opacity="0.48"/>
      <stop offset="100%" stop-color="#00d4aa" stop-opacity="0.62"/>
    </linearGradient>
    <filter id="softShadow" x="-20%" y="-20%" width="140%" height="140%">
      <feDropShadow dx="0" dy="10" stdDeviation="14" flood-color="#000000" flood-opacity="0.34"/>
    </filter>
  </defs>

  <rect width="660" height="400" fill="#0a0d12"/>
  <rect width="660" height="400" fill="url(#glow)"/>

  <g opacity="0.18">
    <path d="M34 72 H626" stroke="#1a2430" stroke-width="1"/>
    <path d="M34 328 H626" stroke="#1a2430" stroke-width="1"/>
    <path d="M82 34 V366" stroke="#1a2430" stroke-width="1"/>
    <path d="M578 34 V366" stroke="#1a2430" stroke-width="1"/>
  </g>

  <g transform="translate(54 54)" filter="url(#softShadow)">
    <circle cx="26" cy="26" r="23" fill="#101721" stroke="#24313c"/>
    <circle cx="26" cy="26" r="8" fill="#00d4aa"/>
    <circle cx="12" cy="16" r="5" fill="#c8fff1"/>
    <circle cx="40" cy="15" r="5" fill="#c8fff1"/>
    <circle cx="14" cy="39" r="5" fill="#c8fff1"/>
    <circle cx="42" cy="38" r="5" fill="#c8fff1"/>
    <path d="M17 18 L26 26 L37 17 M18 37 L26 26 L38 36" fill="none" stroke="#00d4aa" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>
  </g>

  <text x="118" y="78" fill="#f3fbff" font-family="Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif" font-size="34" font-weight="700" letter-spacing="0">NodeNet</text>
  <text x="120" y="104" fill="#7f909d" font-family="Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif" font-size="14" font-weight="500" letter-spacing="0">Network tooling, neatly packaged</text>

  <g transform="translate(180 200)">
    <circle r="52" fill="#111922" stroke="#1e2a34" stroke-width="1.5" opacity="0.62"/>
  </g>

  <g transform="translate(480 200)">
    <circle r="52" fill="#111922" stroke="#1e2a34" stroke-width="1.5" opacity="0.62"/>
  </g>

  <g fill="none" stroke-linecap="round" stroke-linejoin="round">
    <path d="M257 200 C296 166 361 166 402 199" stroke="#1b2731" stroke-width="18" opacity="0.38"/>
    <path d="M260 200 C299 169 359 169 398 198" stroke="url(#arrow)" stroke-width="5"/>
    <path d="M387 181 L403 199 L379 203" stroke="#00d4aa" stroke-width="5" opacity="0.62"/>
  </g>

  <text x="330" y="126" text-anchor="middle" fill="#a5b2bc" font-family="Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif" font-size="18" font-weight="650" letter-spacing="0">Drag to Applications</text>
  <text x="330" y="151" text-anchor="middle" fill="#596873" font-family="Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif" font-size="13" font-weight="500" letter-spacing="0">Install NodeNet for all your macOS workflows</text>

  <text x="614" y="354" text-anchor="end" fill="#465560" font-family="Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif" font-size="12" font-weight="600" letter-spacing="0">${escapeXml(version)}</text>
</svg>`;

await mkdir(path.dirname(outputPath), { recursive: true });
await sharp(Buffer.from(svg)).png().toFile(outputPath);

console.log(`Generated ${outputPath} (${width}x${height})`);
