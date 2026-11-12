import { expect, test, type Page } from '@playwright/test';

// Search needs an API key, so this suite runs against a stand-in GIPHY that e2e/serve.mjs starts.
test.describe.configure({ mode: 'serial' });

async function login(page: Page) {
  await page.goto('/');
  await expect(page).toHaveURL(/\/login$/);
  await page.getByPlaceholder('Username').fill('e2e');
  await page.getByPlaceholder('Password').fill('e2e-password-1');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('button', { name: 'Upload' })).toBeVisible();
}

test('add an API key in Settings, search, and save a result to the library', async ({ page }) => {
  await login(page);

  // Without a key the Search tab says what to do.
  await page.getByRole('link', { name: 'Search' }).click();
  await expect(page).toHaveURL(/\/search/);
  await expect(page.getByText('KLIPY needs an API key.')).toBeVisible();

  // A wrong key is saved, and the provider's complaint shows up when searching.
  await page.getByRole('link', { name: 'Open Settings' }).click();
  await expect(page).toHaveURL(/\/settings$/);
  const card = page.getByTestId('search-settings');
  const giphy = card.locator('form', { hasText: 'GIPHY' });
  await card.getByLabel('GIPHY API key').fill('wrong-key');
  await giphy.getByRole('button', { name: 'Save' }).click();
  await expect(giphy.getByText('Key saved')).toBeVisible();

  await page.getByRole('link', { name: 'Search' }).click();
  await page.getByRole('tab', { name: 'GIPHY' }).click();
  await expect(page.getByRole('alert').getByText('rejected the API key')).toBeVisible();

  // The right key, entered over the wrong one.
  await page.getByRole('link', { name: 'Settings' }).click();
  await card.getByLabel('GIPHY API key').fill('e2e-giphy-key');
  await giphy.getByRole('button', { name: 'Save' }).click();
  await expect(giphy.getByText('Key saved')).toBeVisible();
  // The key is never shown again.
  await expect(card.getByLabel('GIPHY API key')).toHaveValue('');

  await page.getByRole('link', { name: 'Search' }).click();
  await page.getByRole('tab', { name: 'GIPHY' }).click();
  await expect(page.getByText('Trending on GIPHY')).toBeVisible();

  await page.getByRole('searchbox', { name: 'Search GIPHY' }).fill('cat');
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/provider=giphy&q=cat/);
  const first = page.getByRole('button', { name: 'Add: cat one' });
  await expect(first).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add: cat two' })).toBeVisible();

  // Add one with a hashtag, one without.
  await page.getByLabel('Hashtags for added GIFs').fill('Cat');
  await first.click({ force: true });
  await expect(page.getByRole('button', { name: 'Added: cat one' })).toBeVisible();
  await page.getByLabel('Hashtags for added GIFs').fill('');
  await page.getByRole('button', { name: 'Add: cat two' }).click({ force: true });
  await expect(page.getByRole('button', { name: 'Added: cat two' })).toBeVisible();

  await expect
    .poll(async () => {
      const body = await (await page.request.get('/api/v1/gifs?status=ready&sort=old')).json();
      return body.items.map((g: { tags: string[]; source: { type: string; id: string } }) => `${g.source.type}:${g.source.id}:${g.tags.join(',')}`).sort();
    })
    .toEqual(['giphy:e2e-one:cat', 'giphy:e2e-two:']);

  // The untagged one waits in Unsorted, like any other new GIF.
  await expect(page.getByTestId('unsorted-count')).toHaveText('1');

  // After a reload a result that is already in the library says so when it is added again.
  await page.reload();
  await page.getByRole('button', { name: 'Add: cat one' }).click({ force: true });
  await expect(page.getByText('Already in your library')).toBeVisible();
});

test('select several results and save them together, with hashtags for the batch', async ({ page }) => {
  await login(page);
  await page.goto('/search?provider=giphy&q=dog');
  await expect(page.getByRole('button', { name: 'Add: dog one' })).toBeVisible();
  // "dog one" and "dog two" are the same files as before; make them new by trashing and purging the library.
  for (const g of (await (await page.request.get('/api/v1/gifs?limit=100')).json()).items as { id: string }[]) {
    await page.request.delete(`/api/v1/gifs/${g.id}`, { headers: { 'X-Unigif-Request': '1' } });
  }
  expect((await page.request.post('/api/v1/gifs/trash/empty', { headers: { 'X-Unigif-Request': '1' } })).ok()).toBeTruthy();

  await page.getByRole('button', { name: 'Select', exact: true }).click();
  await page.getByRole('button', { name: 'Select: dog one' }).click({ force: true });
  await page.getByRole('button', { name: 'Select: dog two' }).click({ force: true, modifiers: ['Shift'] });
  await expect(page.getByTestId('selected-count')).toHaveText('2 selected');
  await page.getByLabel('Hashtags for added GIFs').fill('dog pack');
  await page.getByRole('button', { name: 'Save 2' }).click();
  await expect(page.getByText('2 saved')).toBeVisible();
  await expect(page.getByTestId('selected-count')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Added: dog one' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Added: dog two' })).toBeVisible();

  await expect
    .poll(async () => {
      const body = await (await page.request.get('/api/v1/gifs?sort=old')).json();
      return body.items.map((g: { tags: string[] }) => [...g.tags].sort().join(',')).sort();
    })
    .toEqual(['dog,pack', 'dog,pack']);
});
