import { test, expect } from '@playwright/test';

test.describe('Navigation', () => {
  test('app loads home screen by default', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByTestId('home-room-url-input')).toBeVisible();
    await expect(page.getByTestId('home-join-button')).toBeVisible();
  });

  test('settings opens as modal overlay', async ({ page }) => {
    await page.goto('/');
    await page.getByTestId('home-settings-button').click();
    // Settings is a modal, home should still be "behind" it
    await expect(page.getByTestId('settings-display-name-input')).toBeVisible();
  });
});
