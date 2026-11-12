import { readFileSync } from 'node:fs';
import { expect, test, type APIRequestContext, type Page } from '@playwright/test';
import { fixtureFiles } from './fixtures';

// M7: live updates, batch select, hashtag management, trash, "often with" suggestions and API keys.
// The tests share one server and one library, so they run in order.
test.describe.configure({ mode: 'serial' });

const port = Number(process.env.E2E_MANAGE_PORT ?? 8796);
const files = fixtureFiles(port);

async function login(page: Page) {
  await page.goto('/');
  await expect(page).toHaveURL(/\/login$/);
  await page.getByPlaceholder('Username').fill('e2e');
  await page.getByPlaceholder('Password').fill('e2e-password-1');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('button', { name: 'Upload' })).toBeVisible();
}

interface GifJson {
  id: string;
  tags: string[];
}

async function makeKey(page: Page, preset: string): Promise<string> {
  const res = await page.request.post('/api/v1/keys', { data: { name: `e2e ${preset}`, preset } });
  expect(res.status()).toBe(201);
  return (await res.json()).key;
}

/** Upload through the API the way a client mod would, waiting until it has been processed. */
async function addViaApi(request: APIRequestContext, file: string): Promise<GifJson> {
  const res = await request.post('/api/v1/gifs?wait=30', {
    data: readFileSync(file),
    headers: { 'Content-Type': 'image/gif' },
  });
  expect([200, 201]).toContain(res.status());
  return res.json();
}

async function allGifs(page: Page): Promise<GifJson[]> {
  return (await (await page.request.get('/api/v1/gifs?sort=old&limit=100')).json()).items;
}

test('live updates: a GIF added through the API shows up in an open Unsorted tab, and leaves when tagged', async ({ page, playwright, baseURL }) => {
  await login(page);
  const key = await makeKey(page, 'uploader');
  const api = await playwright.request.newContext({ baseURL, extraHTTPHeaders: { Authorization: `Bearer ${key}` } });

  await page.goto('/unsorted');
  await expect(page.getByText('All sorted.')).toBeVisible();

  const gif = await addViaApi(api, files[0]);
  // No reload: the tile and the badge arrive over the event stream.
  await expect(page.getByRole('button', { name: 'GIF without hashtags' })).toHaveCount(1);
  await expect(page.getByTestId('unsorted-count')).toHaveText('1');

  // Tagging it from somewhere else takes it out of the open list.
  const tagged = await page.request.patch(`/api/v1/gifs/${gif.id}`, { data: { tags: ['live'] } });
  expect(tagged.ok()).toBeTruthy();
  await expect(page.getByRole('button', { name: 'GIF without hashtags' })).toHaveCount(0);
  await expect(page.getByTestId('unsorted-count')).toHaveCount(0);
  await api.dispose();
});

test('batch select: add hashtags, remove one, trash with undo', async ({ page, playwright, baseURL }) => {
  await login(page);
  const key = await makeKey(page, 'uploader');
  const api = await playwright.request.newContext({ baseURL, extraHTTPHeaders: { Authorization: `Bearer ${key}` } });
  for (const f of files.slice(1, 4)) await addViaApi(api, f);
  await api.dispose();

  await page.goto('/unsorted');
  await expect(page.getByRole('button', { name: 'GIF without hashtags' })).toHaveCount(3);

  // Select mode: tiles toggle instead of opening. Shift-click ticks a range.
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  const tiles = page.getByRole('button', { name: 'GIF without hashtags' });
  await tiles.nth(0).click();
  await tiles.nth(2).click({ modifiers: ['Shift'] });
  await expect(page.getByTestId('selected-count')).toHaveText('3 selected');
  await tiles.nth(1).click();
  await expect(page.getByTestId('selected-count')).toHaveText('2 selected');
  await page.getByRole('button', { name: 'Select all loaded' }).click();
  await expect(page.getByTestId('selected-count')).toHaveText('3 selected');

  // Add two hashtags to all three; they leave Unsorted.
  await page.getByRole('button', { name: 'Add hashtags' }).click();
  await expect(page.getByRole('dialog', { name: 'Add hashtags to selected GIFs' })).toBeVisible();
  await page.keyboard.type('batch pile');
  await page.keyboard.press('Enter');
  await expect(page.getByText('Added to 3 GIFs')).toBeVisible();
  await expect(page.getByRole('button', { name: 'GIF without hashtags' })).toHaveCount(0);
  await expect
    .poll(async () => (await allGifs(page)).filter((g) => g.tags.includes('batch') && g.tags.includes('pile')).length)
    .toBe(3);

  // Library: pick two, take "pile" off.
  await page.goto('/?q=batch');
  const lib = page.getByRole('button', { name: /GIF tagged #batch #pile/ });
  await expect(lib).toHaveCount(3);
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await lib.nth(0).click();
  await lib.nth(1).click();
  await page.getByRole('button', { name: 'Remove hashtags' }).click();
  const dialog = page.getByRole('dialog', { name: 'Remove hashtags from selected GIFs' });
  await dialog.getByRole('button', { name: /#pile/ }).click();
  await dialog.getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(page.getByText('Removed from 2 GIFs')).toBeVisible();
  await expect
    .poll(async () => (await allGifs(page)).filter((g) => g.tags.includes('pile')).length)
    .toBe(1);

  // Trash all three, then undo.
  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'Select all loaded' }).click();
  await expect(page.getByTestId('selected-count')).toHaveText('3 selected');
  await page.getByRole('button', { name: 'Move to trash' }).click();
  await expect(page.getByText('3 GIFs moved to trash')).toBeVisible();
  await expect(page.getByRole('button', { name: /GIF tagged/ })).toHaveCount(0);
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(page.getByRole('button', { name: /GIF tagged #batch/ })).toHaveCount(3);
});

test('hashtags page: a clashing rename offers a merge; rename and delete work', async ({ page }) => {
  await login(page);
  const [a, b] = (await allGifs(page)).filter((g) => g.tags.includes('batch'));
  expect((await page.request.patch(`/api/v1/gifs/${a.id}`, { data: { tags: ['batch', 'colour'] } })).ok()).toBeTruthy();
  expect((await page.request.patch(`/api/v1/gifs/${b.id}`, { data: { tags: ['batch', 'color'] } })).ok()).toBeTruthy();

  await page.getByRole('link', { name: 'Settings' }).click();
  await page.getByRole('link', { name: 'Hashtags', exact: true }).click();
  await expect(page).toHaveURL(/\/tags$/);
  await expect(page.getByTestId('hashtag-list')).toContainText('#colour');

  // Renaming onto an existing hashtag fails with an offer to merge instead.
  await page.getByRole('button', { name: 'Rename #colour' }).click();
  await page.getByRole('textbox', { name: 'New name for #colour' }).fill('color');
  await page.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText(/already exists/);
  await page.getByRole('button', { name: 'Merge into it instead' }).click();
  await page.getByRole('button', { name: 'Merge', exact: true }).click();
  await expect(page.getByText('#colour merged into #color')).toBeVisible();
  await expect(page.getByTestId('hashtag-list')).not.toContainText('#colour');
  expect((await allGifs(page)).find((g) => g.id === a.id)?.tags).toEqual(['batch', 'color']);

  // A plain rename updates every GIF that has the hashtag.
  await page.getByRole('button', { name: 'Rename #color' }).click();
  await page.getByRole('textbox', { name: 'New name for #color' }).fill('hue');
  await page.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(page.getByText('#color is now #hue')).toBeVisible();
  await expect
    .poll(async () => (await allGifs(page)).filter((g) => g.tags.includes('hue')).length)
    .toBe(2);

  // Delete asks first, then takes the hashtag off every GIF (the GIFs stay).
  await page.getByRole('button', { name: 'Delete #hue' }).click();
  await expect(page.getByText(/Remove #hue from 2 GIFs/)).toBeVisible();
  await page.getByRole('button', { name: 'Remove hashtag' }).click();
  await expect(page.getByTestId('hashtag-list')).not.toContainText('#hue');
  expect((await allGifs(page)).some((g) => g.tags.includes('hue'))).toBe(false);
});

test('trash: a deleted GIF can be picked and restored', async ({ page }) => {
  await login(page);
  const victim = (await allGifs(page)).find((g) => g.tags.includes('live'))!;
  expect((await page.request.delete(`/api/v1/gifs/${victim.id}`, { headers: { 'X-Unigif-Request': '1' } })).status()).toBe(204);

  await page.goto('/trash');
  const tile = page.getByRole('button', { name: /GIF tagged #live/ });
  await expect(tile).toHaveCount(1);
  await tile.click();
  await expect(page.getByTestId('selected-count')).toHaveText('1 selected');
  await page.getByRole('button', { name: 'Restore' }).click();
  await expect(page.getByText('1 GIF restored')).toBeVisible();
  await expect(tile).toHaveCount(0);
  await expect
    .poll(async () => (await allGifs(page)).some((g) => g.id === victim.id))
    .toBe(true);
});

test('tag sheet: "often with" suggests hashtags that sit next to the chosen ones', async ({ page }) => {
  await login(page);
  const [a, b] = (await allGifs(page)).filter((g) => g.tags.includes('batch'));
  for (const g of [a, b]) {
    expect((await page.request.patch(`/api/v1/gifs/${g.id}`, { data: { tags: ['batch', 'sidekick'] } })).ok()).toBeTruthy();
  }
  const key = await makeKey(page, 'uploader');
  const res = await page.request.post('/api/v1/gifs?wait=30', {
    data: readFileSync(files[4]),
    headers: { 'Content-Type': 'image/gif', Authorization: `Bearer ${key}` },
  });
  expect(res.ok()).toBeTruthy();

  await page.goto('/unsorted');
  await page.getByRole('button', { name: 'Start sorting' }).click();
  await expect(page.getByRole('combobox', { name: 'Hashtags' })).toBeFocused();
  await expect(page.getByTestId('often-with')).toHaveCount(0);
  await page.keyboard.type('batch ');
  const often = page.getByTestId('often-with');
  await expect(often.getByRole('button', { name: '#sidekick' })).toBeVisible();
  await often.getByRole('button', { name: '#sidekick' }).click();
  await expect(page.getByRole('button', { name: 'Remove #sidekick' })).toBeVisible();
  await page.keyboard.press('Escape');
});

test('API keys: the secret is shown once, the key works, revoking stops it', async ({ page, playwright, baseURL }) => {
  await login(page);
  await page.goto('/keys');
  await page.getByLabel('Name', { exact: true }).fill('Alex');
  await page.getByLabel(/Hidden hashtags/).first().fill('#sidekick');
  await page.getByRole('button', { name: 'Create key' }).click();

  const secret = page.getByTestId('new-key-value');
  await expect(secret).toBeVisible();
  const key = (await secret.textContent())!.trim();
  expect(key).toMatch(/^ugf_/);
  await expect(page.getByTestId('key-list')).toContainText('Alex');
  await expect(page.getByTestId('key-list')).toContainText('#sidekick');

  const api = await playwright.request.newContext({ baseURL, extraHTTPHeaders: { Authorization: `Bearer ${key}` } });
  expect((await api.get('/api/v1/gifs')).status()).toBe(200);
  // The key cannot see GIFs carrying its hidden hashtag.
  const seen = (await (await api.get('/api/v1/gifs?limit=100')).json()).items as GifJson[];
  expect(seen.some((g) => g.tags.includes('sidekick'))).toBe(false);

  // After a reload the secret is gone for good.
  await page.reload();
  await expect(page.getByTestId('new-key')).toHaveCount(0);
  await expect(page.getByTestId('key-list')).toContainText('Alex');
  expect(await page.content()).not.toContain(key);

  await page.getByRole('button', { name: 'Revoke Alex' }).click();
  await page.getByRole('button', { name: 'Revoke key' }).click();
  await expect(page.getByText('Alex revoked')).toBeVisible();
  await expect(page.getByTestId('key-list')).not.toContainText('Alex');
  expect((await api.get('/api/v1/gifs')).status()).toBe(401);
  await api.dispose();
});

test('detail view: the description is editable alt text, and sizes show stored against original', async ({ page }) => {
  await login(page);
  const g = (await allGifs(page)).find((x) => x.tags.includes('sidekick'))!;
  await page.goto(`/?gif=${g.id}`);
  const dialog = page.getByRole('dialog', { name: 'GIF details' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId('sizes')).toHaveText(/\d+ (B|kB|MB) stored · \d+(\.\d)? (B|kB|MB) original/);

  const field = dialog.getByTestId('description');
  await field.fill('A test card scrolling sideways');
  await field.press('Enter');
  await expect
    .poll(async () => (await (await page.request.get(`/api/v1/gifs/${g.id}`)).json()).description)
    .toBe('A test card scrolling sideways');
  // Escape inside the field undoes the edit and keeps the dialog open.
  await field.fill('something else');
  await field.press('Escape');
  await expect(field).toHaveValue('A test card scrolling sideways');
  await expect(dialog).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  // The tile is labelled with it.
  await expect(page.getByRole('button', { name: /^A test card scrolling sideways, tagged/ })).toHaveCount(1);
});

test('trash: "Empty trash" says how many go, and deletes them for good', async ({ page }) => {
  await login(page);
  const victims = (await allGifs(page)).filter((g) => g.tags.includes('batch')).slice(0, 2);
  for (const v of victims) {
    expect((await page.request.delete(`/api/v1/gifs/${v.id}`, { headers: { 'X-Unigif-Request': '1' } })).status()).toBe(204);
  }
  await page.goto('/trash');
  await expect(page.getByRole('button', { name: /tagged #/ })).toHaveCount(2);
  const box = page.getByTestId('empty-trash');
  await box.getByRole('button', { name: 'Empty trash' }).click();
  await expect(box).toContainText('Delete 2 GIFs forever?');
  // Cancel keeps everything.
  await box.getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByRole('button', { name: /tagged #/ })).toHaveCount(2);

  await box.getByRole('button', { name: 'Empty trash' }).click();
  await box.getByRole('button', { name: 'Delete forever' }).click();
  await expect(page.getByText('2 GIFs deleted for good')).toBeVisible();
  await expect(page.getByText('The trash is empty.')).toBeVisible();
  await expect(page.getByTestId('empty-trash')).toHaveCount(0);
  for (const v of victims) expect((await page.request.get(`/api/v1/gifs/${v.id}`)).status()).toBe(404);
});

test('settings: clear cache, list and revoke sessions, change the password', async ({ page, browser, baseURL }) => {
  await login(page);
  // Something in the cache: ask for a GIF conversion once.
  const g = (await allGifs(page))[0];
  expect((await page.request.get(`/m/${g.id}/full.gif`)).status()).toBe(200);

  // A second device signs in.
  const other = await browser.newContext({ baseURL });
  const otherPage = await other.newPage();
  await login(otherPage);

  await page.goto('/settings');
  await page.getByRole('button', { name: 'Clear cache' }).click();
  await expect(page.getByText(/Cleared 1 converted file/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Clear cache' })).toBeDisabled();

  const sessions = page.getByTestId('sessions');
  await expect(sessions.getByText('This device')).toHaveCount(1);
  const revokeButtons = sessions.getByRole('button', { name: 'Revoke' });
  const before = await revokeButtons.count();
  expect(before).toBeGreaterThanOrEqual(1);
  // Revoke every other session: the second device is signed out.
  for (let n = before; n > 0; n--) {
    await revokeButtons.first().click();
    await expect(revokeButtons).toHaveCount(n - 1);
  }
  expect((await otherPage.request.get('/api/v1/me')).status()).toBe(401);

  // Change the password, check the old one no longer works, then change it back.
  const form = page.getByTestId('password-form');
  await form.getByLabel('Current password').fill('wrong-password');
  await form.getByLabel('New password', { exact: true }).fill('e2e-password-2');
  await form.getByLabel('Repeat new password').fill('e2e-password-3');
  await expect(form.getByRole('alert')).toHaveText('The new passwords do not match.');
  await expect(form.getByRole('button', { name: 'Change password' })).toBeDisabled();
  await form.getByLabel('Repeat new password').fill('e2e-password-2');
  await form.getByRole('button', { name: 'Change password' }).click();
  await expect(form.getByRole('alert')).toHaveText('The current password is wrong.'); // the current password was wrong
  await expect(page).toHaveURL(/\/settings$/); // and that is not a lost session

  await form.getByLabel('Current password').fill('e2e-password-1');
  await form.getByRole('button', { name: 'Change password' }).click();
  await expect(page.getByText('Password changed. Every other device was signed out.')).toBeVisible();
  const old = await otherPage.request.post('/api/v1/auth/login', { data: { username: 'e2e', password: 'e2e-password-1' } });
  expect(old.status()).toBe(401);
  expect((await page.request.get('/api/v1/me')).status()).toBe(200); // this session stays

  await form.getByLabel('Current password').fill('e2e-password-2');
  await form.getByLabel('New password', { exact: true }).fill('e2e-password-1');
  await form.getByLabel('Repeat new password').fill('e2e-password-1');
  await form.getByRole('button', { name: 'Change password' }).click();
  await expect(form.getByLabel('Current password')).toHaveValue('');
  const back = await otherPage.request.post('/api/v1/auth/login', { data: { username: 'e2e', password: 'e2e-password-1' } });
  expect(back.status()).toBe(200);
  await other.close();
});
