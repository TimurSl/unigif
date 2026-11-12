import { readFileSync } from 'node:fs';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { fixtureFiles } from './fixtures';

// The phone project: the same sorting flow as the desktop test, with touch instead of the keyboard.
// The tests share one server and one library, so they run in order.
test.describe.configure({ mode: 'serial' });

const PORT = Number(process.env.E2E_PHONE_PORT ?? 8798);
const fixtures = () => fixtureFiles(PORT);

async function login(page: Page) {
  await page.goto('/');
  await expect(page).toHaveURL(/\/login$/);
  await page.getByPlaceholder('Username').fill('e2e');
  await page.getByPlaceholder('Password').fill('e2e-password-1');
  await page.getByRole('button', { name: 'Sign in' }).tap();
  await expect(tabBar(page)).toBeVisible();
}

const tabBar = (page: Page) => page.getByRole('navigation', { name: 'Main' });

/** A real touch gesture (Chromium DevTools input), so pointer events behave as on a device. */
async function swipe(page: Page, target: Locator, dx: number, dy: number) {
  const box = (await target.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y, id: 1 }] });
  for (let i = 1; i <= 6; i++) {
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: x + (dx * i) / 6, y: y + (dy * i) / 6, id: 1 }] });
  }
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [{ x: x + dx, y: y + dy, id: 1 }] });
  await cdp.detach();
  // Chromium swallows a tap that follows a synthetic swipe too closely (it reads it as part of the fling).
  await page.waitForTimeout(800);
}

async function longPress(page: Page, target: Locator, ms = 700) {
  const box = (await target.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y, id: 1 }] });
  await page.waitForTimeout(ms);
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  await cdp.detach();
  await page.waitForTimeout(800); // see swipe(): a tap straight after synthetic touch is swallowed
}

async function tagsOf(page: Page): Promise<string[]> {
  const body = await (await page.request.get('/api/v1/gifs?sort=old')).json();
  return body.items.map((g: { tags: string[] }) => g.tags.join(' ')).sort();
}

const readyCount = async (page: Page, query = 'status=ready') =>
  (await (await page.request.get(`/api/v1/gifs?${query}`)).json()).items.length as number;

test('the app is installable: manifest, icons, share target and service worker', async ({ page, request }) => {
  const res = await request.get('/manifest.webmanifest');
  expect(res.headers()['content-type']).toContain('application/manifest+json');
  const manifest = await res.json();
  expect(manifest.display).toBe('standalone');
  expect(manifest.share_target).toMatchObject({ action: '/share', method: 'POST', enctype: 'multipart/form-data' });
  expect(manifest.share_target.params.files[0].name).toBe('file');
  for (const icon of manifest.icons) expect((await request.get(icon.src)).ok(), icon.src).toBe(true);
  expect(manifest.icons.some((i: { purpose: string }) => i.purpose === 'maskable')).toBe(true);

  await page.goto('/login');
  await expect(page.locator('link[rel=manifest]')).toHaveAttribute('href', '/manifest.webmanifest');
  await page.waitForFunction(() => navigator.serviceWorker.controller !== null);
});

test('phone: add files, then sort three GIFs with taps and swipes', async ({ page }) => {
  await login(page);

  // Add tab -> Choose files opens the system file picker.
  await tabBar(page).getByRole('button', { name: 'Add' }).tap();
  await expect(page.getByRole('dialog', { name: 'Add GIFs' })).toBeVisible();
  const chooser = page.waitForEvent('filechooser');
  await page.getByRole('button', { name: 'Choose files' }).tap();
  await (await chooser).setFiles(fixtures().slice(0, 3));
  await expect(page.getByRole('dialog', { name: 'Add GIFs' })).toHaveCount(0);

  await expect.poll(() => readyCount(page)).toBe(3);
  await expect(page.getByTestId('unsorted-count')).toHaveText('3');

  await tabBar(page).getByRole('link', { name: /Unsorted/ }).tap();
  await expect(page).toHaveURL(/\/unsorted$/);
  await page.getByRole('button', { name: 'Start sorting' }).tap();

  // Full screen, GIF at most 40 % of the height.
  const sheet = page.getByRole('dialog', { name: /Unsorted · 1 of 3/ });
  await expect(sheet).toBeVisible();
  const viewport = page.viewportSize()!;
  const sheetBox = (await sheet.boundingBox())!;
  expect(sheetBox.width).toBeCloseTo(viewport.width, 0);
  expect(sheetBox.height).toBeCloseTo(viewport.height, 0);
  const stage = (await page.locator('.stage').boundingBox())!;
  expect(stage.height).toBeLessThanOrEqual(sheetBox.height * 0.4 + 1);
  // The Save row is pinned at the bottom edge.
  const save = (await page.getByRole('button', { name: /^Save/ }).boundingBox())!;
  expect(save.y + save.height).toBeGreaterThan(viewport.height - 24);
  expect(save.height).toBeGreaterThanOrEqual(44);

  // GIF 1: type, tap Save.
  const input = page.getByRole('combobox', { name: 'Hashtags' });
  await expect(input).toBeFocused();
  await expect(input).toHaveAttribute('enterkeyhint', 'done');
  await expect(input).toHaveAttribute('autocapitalize', 'off');
  await page.keyboard.type('cat');
  await page.getByRole('button', { name: /^Save/ }).tap();
  await expect(page.getByRole('dialog', { name: /Unsorted · 2 of 3/ })).toBeVisible();

  // GIF 2: swipe left skips, swipe right comes back.
  const player = page.locator('.player');
  await swipe(page, player, -150, 0);
  await expect(page.getByRole('dialog', { name: /Unsorted · 3 of 3/ })).toBeVisible();
  await swipe(page, player, 150, 0);
  await expect(page.getByRole('dialog', { name: /Unsorted · 2 of 3/ })).toBeVisible();

  // "Same as last" and a typed tag; the keyboard's Done key (Enter) saves.
  await page.getByRole('button', { name: /Same as last/ }).tap();
  await expect(page.getByRole('button', { name: 'Remove #cat' })).toBeVisible();
  await page.keyboard.type('phone ');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', { name: /Unsorted · 3 of 3/ })).toBeVisible();

  // GIF 3: a recent chip, then Save.
  await page.getByRole('button', { name: '#phone', exact: true }).tap();
  await page.getByRole('button', { name: /^Save/ }).tap();
  await expect(page.getByRole('dialog', { name: 'All sorted' })).toBeVisible();
  await page.getByRole('button', { name: 'Close' }).tap();
  await expect(page.getByRole('dialog')).toHaveCount(0);

  await expect(page.getByTestId('unsorted-count')).toHaveCount(0);
  await expect.poll(() => tagsOf(page)).toEqual(['cat', 'cat phone', 'phone']);
});

test('phone: swipe down closes the tag sheet and keeps the GIF unsorted', async ({ page }) => {
  await login(page);
  await page.getByTestId('file-input').setInputFiles(fixtures()[3]);
  await expect(page.getByTestId('unsorted-count')).toHaveText('1');
  await expect.poll(() => readyCount(page, 'status=ready&unsorted=true')).toBe(1);

  await tabBar(page).getByRole('link', { name: /Unsorted/ }).tap();
  await page.getByRole('button', { name: 'Start sorting' }).tap();
  await expect(page.getByRole('dialog', { name: /Unsorted/ })).toBeVisible();
  await swipe(page, page.locator('.player'), 0, 220);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByTestId('unsorted-count')).toHaveText('1');
});

test('phone: long-press a tile for the menu: edit hashtags, then delete with undo', async ({ page }) => {
  await login(page);
  const tile = page.getByRole('button', { name: /GIF tagged #cat #phone/ });
  await expect(tile).toHaveCount(1);

  // A short tap still opens the detail view; the menu only comes from a long press.
  await tile.tap();
  await expect(page.getByRole('dialog', { name: 'GIF details' })).toBeVisible();
  await page.getByRole('button', { name: 'Close' }).tap();
  await expect(page.getByRole('dialog')).toHaveCount(0);

  await longPress(page, tile);
  const menu = page.getByRole('dialog', { name: /Actions for/ });
  await expect(menu).toBeVisible();
  // The long press did not also open the detail view.
  await expect(page.getByRole('dialog', { name: 'GIF details' })).toHaveCount(0);
  await expect(menu.getByRole('button', { name: 'Copy link' })).toBeVisible();

  await menu.getByRole('button', { name: 'Edit hashtags' }).tap();
  const edit = page.getByRole('dialog', { name: 'Edit hashtags' });
  await expect(edit).toBeVisible();
  await page.keyboard.type('menu');
  await page.keyboard.press('Enter');
  await expect(edit).toHaveCount(0);
  await expect(page.getByRole('button', { name: /GIF tagged #cat #phone #menu/ })).toHaveCount(1);

  await longPress(page, page.getByRole('button', { name: /GIF tagged #cat #phone #menu/ }));
  await page.getByRole('dialog', { name: /Actions for/ }).getByRole('button', { name: 'Delete' }).tap();
  await expect(page.getByText('Moved to trash')).toBeVisible();
  await expect(page.getByRole('button', { name: /GIF tagged #cat #phone #menu/ })).toHaveCount(0);
  await page.getByRole('button', { name: 'Undo' }).tap();
  await expect(page.getByRole('button', { name: /GIF tagged #cat #phone #menu/ })).toHaveCount(1);
});

test('phone: a GIF shared to the installed app lands in Unsorted', async ({ page }) => {
  await login(page);
  // The service worker must control the page, as it does in an installed app.
  await page.waitForFunction(() => navigator.serviceWorker.controller !== null);
  const before = await readyCount(page, 'status=ready&unsorted=true');

  // What Android does when you share a GIF to the app: a multipart form POST to the manifest's action.
  const bytes = readFileSync(fixtures()[4]).toString('base64');
  await page.evaluate((b64) => {
    const raw = atob(b64);
    const data = Uint8Array.from(raw, (c) => c.charCodeAt(0));
    const form = document.createElement('form');
    form.method = 'POST';
    form.action = '/share';
    form.enctype = 'multipart/form-data';
    const file = document.createElement('input');
    file.type = 'file';
    file.name = 'file';
    const transfer = new DataTransfer();
    transfer.items.add(new File([data], 'shared.gif', { type: 'image/gif' }));
    file.files = transfer.files;
    form.append(file);
    document.body.append(form);
    form.submit();
  }, bytes);

  await expect(page).toHaveURL(/\/unsorted$/);
  await expect(page.getByTestId('unsorted-count')).toHaveText(String(before + 1));
  await expect.poll(() => readyCount(page, 'status=ready&unsorted=true')).toBe(before + 1);
  // The stash is emptied: reloading does not upload it again.
  await page.reload();
  await expect(page.getByTestId('unsorted-count')).toHaveText(String(before + 1));
});

test('phone: shared text without a link says so instead of failing silently', async ({ page }) => {
  await login(page);
  await page.waitForFunction(() => navigator.serviceWorker.controller !== null);
  // Shared text without a link says so instead of failing silently.
  await page.evaluate(() => {
    const form = document.createElement('form');
    form.method = 'POST';
    form.action = '/share';
    form.enctype = 'multipart/form-data';
    const text = document.createElement('input');
    text.name = 'text';
    text.value = 'just some words';
    form.append(text);
    document.body.append(form);
    form.submit();
  });
  await expect(page).toHaveURL(/\/unsorted$/);
  await expect(page.getByText('the shared text had no link')).toBeVisible();
});

test('phone: the tab bar has Library, Unsorted, Search, Add and Settings; Settings can sign out', async ({ page }) => {
  await login(page);
  const bar = tabBar(page);
  for (const name of ['Library', 'Unsorted', 'Search', 'Add', 'Settings']) {
    const item = bar.getByText(name, { exact: true });
    await expect(item).toBeVisible();
    expect((await item.locator('xpath=ancestor-or-self::a|ancestor-or-self::button').first().boundingBox())!.height).toBeGreaterThanOrEqual(44);
  }
  // Hashtag browsing sits under the search box.
  await bar.getByRole('link', { name: 'Library' }).tap();
  await expect(page.getByRole('navigation', { name: 'Browse hashtags' }).getByRole('link', { name: '#cat' })).toBeVisible();

  await bar.getByRole('link', { name: 'Settings' }).tap();
  await expect(page).toHaveURL(/\/settings$/);
  await expect(page.getByText('Signed in as')).toBeVisible();
  await page.getByRole('button', { name: 'Sign out' }).tap();
  await expect(page).toHaveURL(/\/login$/);
});

test('phone: long-press → Select, tick more tiles, add a hashtag to all; the new pages fit the screen', async ({ page }) => {
  await login(page);
  const tiles = page.getByRole('button', { name: /^GIF tagged/ });
  expect(await tiles.count()).toBeGreaterThanOrEqual(2);

  await longPress(page, tiles.first());
  await page.getByRole('dialog', { name: /Actions for/ }).getByRole('button', { name: 'Select', exact: true }).tap();
  await expect(page.getByTestId('selected-count')).toHaveText('1 selected');
  await tiles.nth(1).tap();
  await expect(page.getByTestId('selected-count')).toHaveText('2 selected');

  // The bar sits above the tab bar and its buttons are touch-sized.
  const bar = page.getByRole('toolbar', { name: 'Selected GIFs' });
  const tab = (await tabBar(page).boundingBox())!;
  const barBox = (await bar.boundingBox())!;
  expect(barBox.y + barBox.height).toBeLessThanOrEqual(tab.y + 1);
  expect((await bar.getByRole('button', { name: 'Add hashtags' }).boundingBox())!.height).toBeGreaterThanOrEqual(44);

  await bar.getByRole('button', { name: 'Add hashtags' }).tap();
  await expect(page.getByRole('dialog', { name: 'Add hashtags to selected GIFs' })).toBeVisible();
  await page.keyboard.type('picked');
  await page.keyboard.press('Enter');
  await expect(page.getByText('Added to 2 GIFs')).toBeVisible();
  await expect
    .poll(async () => (await tagsOf(page)).filter((t) => t.split(' ').includes('picked')).length)
    .toBe(2);

  // Settings leads to the new pages, and none of them scrolls sideways.
  await tabBar(page).getByRole('link', { name: 'Settings' }).tap();
  for (const [link, heading] of [
    ['Hashtags', 'Hashtags'],
    ['Trash', 'Trash'],
    ['API keys', 'API keys'],
  ] as const) {
    await page.getByTestId('manage-card').getByRole('link', { name: link, exact: true }).tap();
    await expect(page.getByRole('heading', { name: heading, exact: true })).toBeVisible();
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow).toBeLessThanOrEqual(0);
    await page.goBack();
    await expect(page.getByTestId('manage-card')).toBeVisible();
  }
});
