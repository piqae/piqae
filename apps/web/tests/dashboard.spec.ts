import { expect, test } from '@playwright/test';

test('operations surface exposes state with semantic navigation', async ({ page }) => {
  await page.goto('/dashboard');
  await expect(page).toHaveTitle('Operations · Piqae');
  await expect(page.getByRole('heading', { name: 'Operations' })).toBeVisible();
  await expect(page.getByText('Demo data — no control-plane requests are being made.')).toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Main navigation' })).toBeVisible();
  await page.getByLabel('Account and workspace: Demo workspace').click();
  const accountMenu = page.locator('.account-menu');
  await expect(accountMenu.getByText('developer@piqae.local')).toBeVisible();
  await expect(accountMenu.getByText('Demo workspace', { exact: true })).toBeVisible();
  await expect(accountMenu.getByRole('link', { name: 'Settings' })).toBeVisible();
  await expect(accountMenu.getByRole('link', { name: 'Sign out' })).toBeVisible();
  await expect(page.getByText('2 uncertain handoffs')).toBeVisible();
  await expect(page.getByRole('table')).toBeVisible();

  // Views are query-string state on the one page, not separate routes.
  await page
    .getByRole('group', { name: 'Switch operational view' })
    .getByRole('button', { name: 'Printers' })
    .click();
  await expect(page).toHaveURL(/\/dashboard\?view=printers$/);
  await expect(page.getByRole('columnheader', { name: 'Printer' })).toBeVisible();
});

test('legacy dashboard routes redirect into the collapsed structure', async ({ request }) => {
  const redirects = [
    ['/dashboard/jobs', '/dashboard?view=jobs'],
    ['/dashboard/printers', '/dashboard?view=printers'],
    ['/dashboard/nodes', '/dashboard?view=nodes'],
    ['/dashboard/agents', '/dashboard?view=nodes'],
    ['/dashboard/jobs/job_01K0VY5YJ', '/dashboard?job=job_01K0VY5YJ'],
    ['/dashboard/printers/prt_01', '/dashboard?printer=prt_01'],
    ['/dashboard/nodes/agt_01', '/dashboard?node=agt_01'],
    ['/dashboard/agents/agt_01', '/dashboard?node=agt_01'],
    ['/dashboard/accounts', '/dashboard?view=customers'],
    ['/dashboard/api-keys', '/dashboard/settings#api-keys'],
    ['/dashboard/webhooks', '/dashboard/settings#webhooks'],
    ['/dashboard/developers', '/dashboard/settings#api-keys'],
    ['/dashboard/settings/team', '/dashboard/settings#team'],
    ['/dashboard/settings/billing', '/dashboard/settings#billing']
  ] as const;

  for (const [from, to] of redirects) {
    const response = await request.get(from, { maxRedirects: 0 });
    expect(response.status(), `${from} should redirect`).toBe(308);
    expect(response.headers()['location'], `${from} target`).toBe(to);
  }
});

test('redirects preserve callback state carried in the query string', async ({ request }) => {
  // Stripe returns to the billing URL with ?checkout=success; dropping it would
  // silently swallow the post-checkout confirmation.
  const response = await request.get('/dashboard/settings/billing?checkout=success', {
    maxRedirects: 0
  });
  expect(response.status()).toBe(308);
  expect(response.headers()['location']).toBe('/dashboard/settings?checkout=success#billing');
});

test('job detail is a deep-linkable drawer', async ({ page }) => {
  await page.goto('/dashboard/jobs/job_01K0VY5YJ');
  await expect(page).toHaveURL(/\/dashboard\?job=job_01K0VY5YJ$/);
  const drawer = page.locator('dialog[aria-labelledby="detail-title"]');
  await expect(drawer).toBeVisible();
  await expect(drawer.getByText('Event timeline')).toBeVisible();
  await expect(drawer.getByRole('heading', { level: 2 })).toBeVisible();
});

test('responsive dashboard remains inside the viewport', async ({ page }, testInfo) => {
  test.skip(!testInfo.project.name.startsWith('mobile'), 'Mobile-only layout assertion');
  await page.goto('/dashboard?view=printers');
  await expect(page.getByRole('navigation', { name: 'Main navigation' })).toBeVisible();
  await expect(page.getByRole('columnheader', { name: 'Printer' })).toBeVisible();
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
  // Integrator accounts default to the safe, aggregate customer view. Node
  // enrolment is an own-workspace action and is intentionally available only
  // after selecting that isolated scope.
  await page.goto('/dashboard?scope=own');
  await page.getByRole('button', { name: 'Add node' }).click();
  const enrolment = page.getByRole('dialog', { name: 'Add a node' });
  await expect(enrolment).toBeVisible();
  await expect(
    enrolment.getByText('Demo mode: preview only. No enrolment will be created.')
  ).toBeVisible();
  await expect(enrolment.getByText('Open the app')).toBeVisible();
  await expect(enrolment.getByText('Advanced options')).toBeVisible();
  await expect(enrolment.getByRole('button', { name: 'Continue in Piqae' })).toBeDisabled();

  // Each settings dialog is opened from a fresh load: the mobile project uses
  // touch emulation, where dismissing one modal before opening the next is
  // needlessly flaky. Dismissal itself is covered by the component test.
  await page.goto('/dashboard/settings');
  await expect(page.getByRole('button', { name: 'Save retention' })).toBeDisabled();
  await page.getByRole('button', { name: 'Create secret key', exact: true }).first().click();
  const apiKey = page.getByRole('dialog', { name: 'Create secret key' });
  await expect(apiKey).toBeVisible();
  await expect(apiKey.getByRole('checkbox', { name: 'Read jobs' })).toBeChecked();
  await expect(
    apiKey.getByRole('button', { name: 'Create secret key', exact: true })
  ).toBeDisabled();
  await expect(
    apiKey.getByRole('button', { name: 'Close secret key dialog', exact: true })
  ).toBeVisible();

  await page.goto('/dashboard/settings');
  await page.getByRole('button', { name: 'Add endpoint' }).click();
  const webhook = page.getByRole('dialog', { name: 'Add webhook endpoint' });
  await expect(webhook).toBeVisible();
  await expect(
    webhook.getByRole('button', { name: 'Create endpoint', exact: true })
  ).toBeDisabled();

  await page.goto('/dashboard?job=job_01K0VY5YJ');
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  const cancellation = page.getByRole('dialog', { name: 'Cancel this print job?' });
  await expect(cancellation).toBeVisible();
  await expect(cancellation.getByRole('button', { name: 'Confirm cancellation' })).toBeDisabled();
});
