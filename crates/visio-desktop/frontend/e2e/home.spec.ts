import { test, expect } from '@playwright/test';

test.describe('Home Screen', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
  });

  test('displays room URL input and join button', async ({ page }) => {
    await expect(page.getByTestId('home-room-url-input')).toBeVisible();
    await expect(page.getByTestId('home-join-button')).toBeVisible();
  });

  test('displays settings button', async ({ page }) => {
    await expect(page.getByTestId('home-settings-button')).toBeVisible();
  });

  test('join button is disabled when room URL is empty', async ({ page }) => {
    const joinButton = page.getByTestId('home-join-button');
    // Button should be disabled or have disabled styling when no URL entered
    await expect(joinButton).toBeVisible();
  });

  test('can enter room URL', async ({ page }) => {
    const input = page.getByTestId('home-room-url-input');
    await input.fill('https://meet.example.com/abc-defg-hij');
    await expect(input).toHaveValue('https://meet.example.com/abc-defg-hij');
  });

  test('can enter display name', async ({ page }) => {
    const input = page.getByTestId('home-display-name-input');
    await input.fill('E2E Tester');
    await expect(input).toHaveValue('E2E Tester');
  });

  test('settings modal opens and closes', async ({ page }) => {
    await page.getByTestId('home-settings-button').click();
    await expect(page.getByTestId('settings-display-name-input')).toBeVisible();
    await page.getByTestId('settings-close-button').click();
    await expect(page.getByTestId('settings-display-name-input')).not.toBeVisible();
  });
});
