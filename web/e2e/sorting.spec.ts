import { expect, test, type Page } from '@playwright/test';
import { fixtureFiles } from './fixtures';

// The tests share one server and one library, so they run in order.
test.describe.configure({ mode: 'serial' });

async function login(page: Page) {
  await page.goto('/');
  await expect(page).toHaveURL(/\/login$/);
  await page.getByPlaceholder('Username').fill('e2e');
  await page.getByPlaceholder('Password').fill('e2e-password-1');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('button', { name: 'Upload' })).toBeVisible();
}

async function tagsOf(page: Page): Promise<string[][]> {
  const res = await page.request.get('/api/v1/gifs?sort=old');
  const body = await res.json();
  return body.items.map((g: { tags: string[] }) => g.tags);
}

test('upload three GIFs and sort them using only the keyboard', async ({ page }) => {
  await login(page);

  // Upload through the file input; the files end up in Unsorted.
  await page.getByTestId('file-input').setInputFiles(fixtureFiles().slice(0, 3));
  await expect(page.getByTestId('unsorted-count')).toHaveText('3');
  await expect
    .poll(async () => (await (await page.request.get('/api/v1/gifs?status=ready')).json()).items.length)
    .toBe(3);

  await page.getByRole('link', { name: /Unsorted/ }).click();
  await expect(page).toHaveURL(/\/unsorted$/);

  // Focus "Start sorting" and press Enter: from here on the mouse is not used.
  const start = page.getByRole('button', { name: 'Start sorting' });
  await start.focus();
  await page.keyboard.press('Enter');

  const sheet = page.getByRole('dialog', { name: /Unsorted · 1 of 3/ });
  await expect(sheet).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Hashtags' })).toBeFocused();

  // GIF 1: Enter with no hashtag does not save.
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status').filter({ hasText: 'Add a hashtag, or Skip' })).toBeVisible();
  await expect(page.getByRole('dialog', { name: /Unsorted · 1 of 3/ })).toBeVisible();

  // Type a hashtag and press Enter: saves and slides to the next GIF with the cursor still in the input.
  await page.keyboard.type('Cat');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', { name: /Unsorted · 2 of 3/ })).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Hashtags' })).toBeFocused();

  // GIF 2: Space turns the text into a chip; two hashtags, then Enter.
  await page.keyboard.type('funny ');
  await page.keyboard.type('#reaction');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', { name: /Unsorted · 3 of 3/ })).toBeVisible();

  // GIF 3: Alt+0 applies the hashtags of the GIF saved last.
  await expect(page.getByRole('combobox', { name: 'Hashtags' })).toBeFocused();
  await page.keyboard.press('Alt+Digit0');
  await expect(page.getByRole('button', { name: 'Remove #funny' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Remove #reaction' })).toBeVisible();
  await page.keyboard.press('Enter');

  await expect(page.getByRole('dialog', { name: 'All sorted' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);

  // Everything left Unsorted and carries the expected hashtags.
  await expect(page.getByTestId('unsorted-count')).toHaveCount(0);
  // (Uploads run in parallel, so which file is first in the queue is not fixed; the set of results is.)
  await expect
    .poll(async () => (await tagsOf(page)).map((t) => t.join(' ')).sort())
    .toEqual(['cat', 'funny reaction', 'funny reaction']);
});

test('library: search by hashtag and open the detail view', async ({ page }) => {
  await login(page);
  await page.getByRole('searchbox', { name: 'Search hashtags' }).fill('cat ');
  await expect(page.getByRole('button', { name: /GIF tagged #cat/ })).toHaveCount(1);
  await page.getByRole('button', { name: /GIF tagged #cat/ }).click();
  await expect(page).toHaveURL(/gif=/);
  await expect(page.getByRole('dialog', { name: 'GIF details' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('wrong password shows an error and stays on the login page', async ({ page }) => {
  await page.goto('/login');
  await page.getByPlaceholder('Username').fill('e2e');
  await page.getByPlaceholder('Password').fill('nope-nope-nope');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('alert')).not.toBeEmpty();
  await expect(page).toHaveURL(/\/login$/);
});

test('suggestions: Tab accepts the top one, Backspace removes the last chip, Shift+Enter flips the checkbox', async ({ page }) => {
  await login(page);
  // The library has hashtags from the sorting test; add a fresh unsorted GIF to tag.
  await page.getByTestId('file-input').setInputFiles(fixtureFiles()[3]);
  await expect(page.getByTestId('unsorted-count')).toHaveText('1');
  await expect
    .poll(async () => (await (await page.request.get('/api/v1/gifs?status=ready&unsorted=true')).json()).items.length)
    .toBe(1);
  await page.goto('/unsorted');
  await page.getByRole('button', { name: 'Start sorting' }).click();
  await expect(page.getByRole('combobox', { name: 'Hashtags' })).toBeFocused();
  await page.keyboard.type('fu');
  await expect(page.getByRole('option', { name: /#funny/ })).toBeVisible();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Remove #funny' })).toBeVisible();
  await page.keyboard.press('Backspace');
  await expect(page.getByRole('button', { name: 'Remove #funny' })).toHaveCount(0);
  await page.keyboard.type('misc');
  // Shift+Enter saves and does the opposite of "proceed" (checked): the sheet closes.
  await page.keyboard.press('Shift+Enter');
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('detail: edit hashtags, then delete with undo', async ({ page }) => {
  await login(page);
  await page.getByRole('searchbox', { name: 'Search hashtags' }).fill('cat ');
  await page.getByRole('button', { name: /GIF tagged #cat/ }).click();
  await page.getByRole('button', { name: 'Edit hashtags' }).click();
  await page.keyboard.type('edited');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', { name: 'Edit hashtags' })).toHaveCount(0);
  await expect(page.getByRole('dialog', { name: 'GIF details' }).getByRole('button', { name: '#edited' })).toBeVisible();

  await page.getByRole('button', { name: 'Delete' }).click();
  await expect(page.getByText('Moved to trash')).toBeVisible();
  await expect(page.getByRole('button', { name: /GIF tagged #cat/ })).toHaveCount(0);
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(page.getByRole('button', { name: /GIF tagged #cat/ })).toHaveCount(1);
});

test('detail: copy a link in every format, open the share page, see the converted copies in Settings', async ({ page }) => {
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
  await login(page);
  await page.getByRole('searchbox', { name: 'Search hashtags' }).fill('cat ');
  await page.getByRole('button', { name: /GIF tagged #cat/ }).click();
  const detail = page.getByRole('dialog', { name: 'GIF details' });
  const copied = () => page.evaluate(() => navigator.clipboard.readText());

  const types: Record<string, string> = { MP4: 'video/mp4', GIF: 'image/gif', WebM: 'video/webm', WebP: 'image/webp' };
  const file: Record<string, string> = { MP4: 'full.mp4', GIF: 'full.gif', WebM: 'full.webm', WebP: 'full.webp' };
  let id = '';
  for (const [label, type] of Object.entries(types)) {
    await detail.getByRole('button', { name: label, exact: true }).click();
    await expect(page.getByRole('status').filter({ hasText: `${label} link copied` })).toBeVisible();
    const url = await copied();
    const m = url.match(/\/m\/([1-9A-HJ-NP-Za-km-z]{12})\/(.+)$/);
    expect(m, url).not.toBeNull();
    expect(m![2]).toBe(file[label]);
    id = m![1];
    // The address works with no credentials at all, and the converted formats are real files.
    const res = await page.context().request.get(url, { headers: { cookie: '' } });
    expect(res.status(), url).toBe(200);
    expect(res.headers()['content-type']).toBe(type);
    expect((await res.body()).length).toBeGreaterThan(100);
  }

  // "Link" is the share page, which unfurls in chats.
  await detail.getByRole('button', { name: 'Link', exact: true }).click();
  const share = await copied();
  expect(share).toMatch(new RegExp(`/g/${id}$`));
  const html = await (await page.request.get(share)).text();
  expect(html).toContain('property="og:video"');
  expect(html).toContain(`/m/${id}/full.mp4`);

  // Settings lists the converted files.
  await page.keyboard.press('Escape');
  await page.goto('/settings');
  await expect(page.getByText(/Converted copies/)).toBeVisible();
  await expect(page.getByText(/[1-9]\d* files, .* of 2\.0 GB/)).toBeVisible();
});
