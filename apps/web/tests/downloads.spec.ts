import { expect, test, type Page } from '@playwright/test';

async function tabTo(page: Page, selector: string): Promise<void> {
  const target = page.locator(selector);
  for (let attempt = 0; attempt < 30; attempt += 1) {
    await page.keyboard.press('Tab');
    if (await target.evaluate((element) => element === document.activeElement)) return;
  }
  throw new Error(`Keyboard focus did not reach ${selector}`);
}

test('downloads remain truthful, responsive, and keyboard reachable', async ({ page }) => {
  for (const viewport of [
    { width: 1024, height: 800 },
    { width: 768, height: 900 }
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/downloads');

    await expect(
      page.getByRole('heading', { level: 1, name: /^(Piqae for .* is almost ready|Download Piqae)$/ })
    ).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Piqae for every printer computer.' })).toBeVisible();
    await expect(page.getByRole('link', { name: /^Download / })).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Ready to print in minutes.' })).toBeVisible();
    await expect(page.getByText('Connect your account')).toBeVisible();
    await expect(
      page.getByRole('heading', { name: 'Everything technical, when you need it.' })
    ).toBeVisible();

    const dimensions = await page.evaluate(() => ({
      documentWidth: document.documentElement.scrollWidth,
      viewportWidth: document.documentElement.clientWidth
    }));
    expect(dimensions.documentWidth).toBeLessThanOrEqual(dimensions.viewportWidth);

    await page.locator('body').focus();
    await tabTo(page, 'a[href="#other-downloads"]');
    await expect(page.locator('a[href="#other-downloads"]')).toBeFocused();
  }
});
