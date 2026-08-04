import { expect, test } from '@playwright/test';

const publicRoutes = [
  ['/', 'Printing infrastructure, ready for your product.'],
  ['/pricing', 'Pay only for prints reported complete.'],
  ['/about', 'Built where every label matters.'],
  ['/compare', 'A simpler print API, without the closed edge.'],
  ['/compare/printnode', 'A familiar print API with a more open operating model.'],
  ['/alternatives/printnode', 'Remote printing that is easier to adopt—and easier to leave.'],
  ['/migrate/printnode', 'Keep the request shape. Change who owns the print path.'],
  ['/open-source', 'Open at the edge. Portable at the core.'],
  ['/security', 'Built for the documents your business depends on.']
] as const;

test('primary public destinations never return a server error', async ({ request }) => {
  const destinations = [
    '/docs',
    '/docs/quickstart',
    '/downloads',
    ...publicRoutes.map(([path]) => path)
  ];
  for (const route of destinations) {
    const response = await request.get(route);
    expect(response.status(), route).toBeLessThan(400);
  }
});

test('the retired product page redirects to useful documentation', async ({ request }) => {
  const response = await request.get('/how-it-works', { maxRedirects: 0 });
  expect(response.status()).toBe(308);
  expect(response.headers().location).toBe('/docs');
});

test('marketing routes have unique content and remain launch-gated', async ({ page }) => {
  for (const [route, heading] of publicRoutes) {
    await page.goto(route);
    await expect(page.getByRole('heading', { level: 1, name: heading })).toBeVisible();
    await expect(page.locator('meta[name="robots"]')).toHaveAttribute('content', 'noindex,nofollow');
    const dimensions = await page.evaluate(() => ({
      documentWidth: document.documentElement.scrollWidth,
      viewportWidth: document.documentElement.clientWidth
    }));
    expect(dimensions.documentWidth).toBeLessThanOrEqual(dimensions.viewportWidth);
  }
});

test('homepage and mobile navigation expose the primary conversion paths', async ({
  page
}, testInfo) => {
  await page.goto('/');
  await expect(
    page.getByRole('img', {
      name: /Piqae dashboard showing node health/
    })
  ).toBeVisible();
  await expect(page.getByRole('link', { name: 'Compare Piqae and PrintNode' })).toBeVisible();
  await expect(
    page.getByRole('heading', {
      name: 'Add full-service printing. Keep building your product.'
    })
  ).toBeVisible();
  await expect(
    page.getByRole('img', {
      name: 'Interactive three-dimensional globe illustrating print jobs travelling through Piqae to local printers'
    })
  ).toBeVisible();
  await expect(page.getByText('Network preview', { exact: true })).toBeVisible();

  if (testInfo.project.name.startsWith('mobile')) {
    await page.getByRole('button', { name: 'Open navigation' }).click();
    await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Pricing', exact: true }).first()).toBeVisible();
  }
});

test('marketing shell supports keyboard navigation at narrow desktop and tablet widths', async ({
  page
}) => {
  await page.setViewportSize({ width: 1024, height: 768 });
  await page.goto('/');
  await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();

  await page.keyboard.press('Tab');
  const skipLink = page.getByRole('link', { name: 'Skip to content' });
  await expect(skipLink).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('#main-content')).toBeFocused();

  await page.setViewportSize({ width: 820, height: 1024 });
  const menu = page.getByRole('button', { name: 'Open navigation' });
  await expect(menu).toBeVisible();
  await menu.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Pricing', exact: true }).first()).toBeVisible();
});

test('cost calculator updates locally and exposes its evidence', async ({ page }) => {
  await page.goto('/tools/printnode-cost-calculator');
  await page.getByLabel('Jobs per month').fill('25000');
  await page.getByLabel('Connected computers / agents').fill('8');
  await expect(page.getByText('Pro', { exact: true })).toBeVisible();
  await expect(page.getByText('Standard Integrator', { exact: true })).toBeVisible();
  await expect(page.getByText('$51 less / month', { exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: 'official USD pricing page' })).toHaveAttribute(
    'href',
    'https://www.printnode.com/en/pricing'
  );
});

test('pricing exposes only the locked Free and Pro catalog', async ({ page }) => {
  await page.goto('/pricing');
  await expect(page.getByRole('heading', { name: 'Piqae Free', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Piqae Pro', exact: true })).toBeVisible();
  const allowanceValues = page.locator('.plan-grid dd');
  await expect(allowanceValues.filter({ hasText: /^100(?:\s|$)/ }).first()).toBeVisible();
  await expect(allowanceValues.filter({ hasText: /^25,000(?:\s|$)/ }).first()).toBeVisible();
  await expect(page.getByText('Launch', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Growth', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Scale', { exact: true })).toHaveCount(0);
});

test('draft comparisons and private routes carry noindex headers', async ({ request }) => {
  for (const route of ['/compare/qz-tray', '/compare/ezeep', '/dashboard', '/auth/session']) {
    const response = await request.get(route, { maxRedirects: 0 });
    expect(response.headers()['x-robots-tag']).toBe('noindex, nofollow');
  }
  const robots = await request.get('/robots.txt');
  await expect(robots).toBeOK();
  expect(await robots.text()).toContain('Disallow: /');
});
