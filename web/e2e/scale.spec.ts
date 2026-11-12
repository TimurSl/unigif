import { expect, test, type Page } from '@playwright/test';

// 50 000 GIFs (rows only; the media requests 404, which is fine for layout and scrolling).
// Runs on demand: E2E_SCALE=1 npm run e2e -- --project scale

async function login(page: Page) {
  await page.goto('/');
  await page.getByPlaceholder('Username').fill('e2e');
  await page.getByPlaceholder('Password').fill('e2e-password-1');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('button', { name: 'Upload' })).toBeVisible();
}

test('the library grid scrolls through thousands of GIFs with a bounded DOM and smooth frames', async ({ page }) => {
  await login(page);
  await expect(page.getByText('50000 GIFs')).toBeVisible();
  const tiles = page.locator('.slot');
  await expect(tiles.first()).toBeVisible();

  // Record frame intervals while scrolling down in steady steps, paging in more as the end nears.
  const result = await page.evaluate(async () => {
    const scroller = document.querySelector('.scroller') as HTMLElement;
    const frames: number[] = [];
    let last = performance.now();
    let running = true;
    const tick = (t: number) => {
      frames.push(t - last);
      last = t;
      if (running) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    let maxSlots = 0;
    const start = performance.now();
    while (performance.now() - start < 8000) {
      scroller.scrollBy(0, 400);
      maxSlots = Math.max(maxSlots, document.querySelectorAll('.slot').length);
      await new Promise((r) => setTimeout(r, 16));
    }
    running = false;
    frames.sort((a, b) => a - b);
    return {
      p95: frames[Math.floor(frames.length * 0.95)],
      frames: frames.length,
      maxSlots,
      scrolled: scroller.scrollTop,
    };
  });
  console.log(`scrolled ${Math.round(result.scrolled)} px, ${result.frames} frames, p95 frame ${result.p95.toFixed(1)} ms, at most ${result.maxSlots} tiles in the DOM`);

  // Virtualised: only tiles near the viewport exist, however far down the list is.
  expect(result.maxSlots).toBeLessThan(150);
  // Pages kept loading: well past the first page of 60 (about 20 rows).
  expect(result.scrolled).toBeGreaterThan(20000);
  // No long stalls while scrolling (a generous bound for a headless CI machine).
  expect(result.p95).toBeLessThan(50);
});

test('typing a search narrows 50 000 GIFs quickly', async ({ page }) => {
  await login(page);
  const box = page.getByRole('searchbox', { name: 'Search hashtags' });
  const started = Date.now();
  await box.fill('tag199 ');
  await expect(page.getByRole('button', { name: /#tag199/ }).first()).toBeVisible();
  expect(Date.now() - started).toBeLessThan(3000);
});
