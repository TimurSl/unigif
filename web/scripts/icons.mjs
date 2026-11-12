// Regenerates the PNG icons in public/ from public/icon.svg (`node scripts/icons.mjs`).
// Uses the Chromium that Playwright already provides; the PNGs are committed, so this only runs
// when the artwork changes.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { chromium } from '@playwright/test';

const pub = join(import.meta.dirname, '..', 'public');
const svg = readFileSync(join(pub, 'icon.svg'), 'utf8');
// Full-bleed variants (no rounded corners) for maskable and Apple icons: the OS applies its own mask.
const square = svg.replace(' rx="112"', '');
// The glyph already sits inside the maskable safe zone (the centre 80 % circle).

const jobs = [
  ['icon-192.png', 192, svg, true],
  ['icon-512.png', 512, svg, true],
  ['icon-maskable-512.png', 512, square, false],
  ['apple-touch-icon.png', 180, square, false],
];

const browser = await chromium.launch();
const page = await browser.newPage();
for (const [name, size, markup, transparent] of jobs) {
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(`<style>html,body{margin:0;background:transparent}svg{display:block;width:${size}px;height:${size}px}</style>${markup}`);
  await page.screenshot({ path: join(pub, name), omitBackground: transparent, clip: { x: 0, y: 0, width: size, height: size } });
  console.log('wrote', name);
}
await browser.close();
