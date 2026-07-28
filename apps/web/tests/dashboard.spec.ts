import { expect, test } from '@playwright/test';

test('dashboard exposes operational state with semantic navigation', async ({ page }) => {
  await page.goto('/dashboard');
  await expect(page).toHaveTitle('Overview · Spool');
  await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible();
  await expect(page.getByText('Demo data — no control-plane requests are being made.')).toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Main navigation' })).toBeVisible();
  await expect(page.getByText('1 uncertain handoff')).toBeVisible();
  await page.goto('/dashboard/jobs');
  await expect(page.getByRole('heading', { name: 'Jobs' })).toBeVisible();
  await expect(page.getByRole('table')).toBeVisible();
});

test('responsive dashboard remains inside the viewport', async ({ page }, testInfo) => {
  test.skip(!testInfo.project.name.startsWith('mobile'), 'Mobile-only layout assertion');
  await page.goto('/dashboard');
  await page.getByRole('button', { name: 'Open navigation' }).click();
  await expect(page.getByRole('navigation', { name: 'Main navigation' })).toBeVisible();
  await page.getByRole('link', { name: 'Printers', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Printers' })).toBeVisible();
  const layout = await page.evaluate(() => {
    const panel = document.querySelector<HTMLElement>('.table-panel');
    return {
      pageOverflow: getComputedStyle(document.body).overflowX,
      panelOverflow: panel ? getComputedStyle(panel).overflowX : null,
      panelContainsWideTable: panel ? panel.scrollWidth > panel.clientWidth : false
    };
  });
  expect(layout).toEqual({
    pageOverflow: 'hidden',
    panelOverflow: 'auto',
    panelContainsWideTable: true
  });
});

test('documentation and hosted authentication boundaries are reachable', async ({ page }) => {
  await page.goto('/docs/quickstart');
  await expect(page.getByRole('heading', { name: 'Print in under ten minutes' })).toBeVisible();
  await expect(page.getByText('A 201 response means durable registration')).toBeVisible();
  await page.goto('/login');
  await expect(page.getByRole('link', { name: /Continue with WorkOS/ })).toHaveAttribute(
    'href',
    '/auth/login?return_to=%2Fdashboard'
  );
});
