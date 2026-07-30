import { expect, test } from '@playwright/test';

test('dashboard exposes operational state with semantic navigation', async ({ page }) => {
  await page.goto('/dashboard');
  await expect(page).toHaveTitle('Overview · Piqae');
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
  await expect(page).toHaveURL(/\/dashboard$/);
  const sessionResponse = await page.request.get('/auth/session');
  const session = await sessionResponse.json();
  expect(JSON.stringify(session)).not.toContain('accessToken');
  expect(JSON.stringify(session)).not.toContain('access_token');
});

test('credential and cancellation dialogs are accessible and non-mutating in demo mode', async ({
  page
}) => {
  await page.goto('/dashboard/agents');
  await expect(page).toHaveURL(/\/dashboard\/nodes$/);
  await page.getByRole('button', { name: 'Add node' }).click();
  const enrolment = page.getByRole('dialog', { name: 'Add a node' });
  await expect(enrolment).toBeVisible();
  await expect(enrolment.getByText('Demo mode: preview only.')).toBeVisible();
  await expect(enrolment.getByText('Browser pairing is recommended')).toBeVisible();
  await expect(enrolment.getByRole('button', { name: 'Create manual token' })).toBeDisabled();

  await page.goto('/dashboard/api-keys');
  await page.getByRole('button', { name: 'Create secret key' }).click();
  const apiKey = page.getByRole('dialog', { name: 'Create secret key' });
  await expect(apiKey).toBeVisible();
  await expect(apiKey.getByRole('checkbox', { name: 'Read jobs' })).toBeChecked();
  await expect(apiKey.getByRole('button', { name: 'Create secret key' })).toBeDisabled();

  await page.goto('/dashboard/webhooks');
  await page.getByRole('button', { name: 'Add endpoint' }).click();
  const webhook = page.getByRole('dialog', { name: 'Add webhook endpoint' });
  await expect(webhook).toBeVisible();
  await expect(webhook.getByRole('button', { name: 'Create endpoint' })).toBeDisabled();
  await expect(page.getByRole('region', { name: 'Demo webhook delivery examples' })).toContainText(
    'Demo only'
  );

  await page.goto('/dashboard/jobs/job_01K0VY5YJ');
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  const cancellation = page.getByRole('dialog', { name: 'Cancel this print job?' });
  await expect(cancellation).toBeVisible();
  await expect(cancellation.getByRole('button', { name: 'Confirm cancellation' })).toBeDisabled();

  await page.goto('/dashboard/settings');
  await expect(page.getByRole('button', { name: 'Save changes' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Save retention' })).toBeDisabled();
});
