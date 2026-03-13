import { test, expect } from '@playwright/test';
import { joinMockRoom } from './tauri-mock';

test.describe('Call Controls', () => {
  test.beforeEach(async ({ page }) => {
    await joinMockRoom(page);
  });

  test('all control buttons are visible', async ({ page }) => {
    await expect(page.getByTestId('call-mic-button')).toBeVisible();
    await expect(page.getByTestId('call-camera-button')).toBeVisible();
    await expect(page.getByTestId('call-screen-share-button')).toBeVisible();
    await expect(page.getByTestId('call-chat-button')).toBeVisible();
    await expect(page.getByTestId('call-hangup-button')).toBeVisible();
  });

  test('mic and camera chevrons are visible', async ({ page }) => {
    await expect(page.getByTestId('call-mic-chevron')).toBeVisible();
    await expect(page.getByTestId('call-camera-chevron')).toBeVisible();
  });

  test('can toggle microphone', async ({ page }) => {
    const micBtn = page.getByTestId('call-mic-button');
    await micBtn.click(); // mute
    await micBtn.click(); // unmute
    // Should not crash
    await expect(micBtn).toBeVisible();
  });

  test('can toggle camera', async ({ page }) => {
    const camBtn = page.getByTestId('call-camera-button');
    await camBtn.click(); // off
    await camBtn.click(); // on
    await expect(camBtn).toBeVisible();
  });

  test('can toggle hand raise via overflow menu', async ({ page }) => {
    // Hand raise is inside the overflow menu, toggled by the "More" button.
    // The "More" button has no data-testid, so locate it by its class and position.
    const overflowTrigger = page.locator('.control-bar .control-btn:not([data-testid])').first();
    await overflowTrigger.click();

    // The overflow menu should now be visible with the hand raise button
    const handBtn = page.getByTestId('call-hand-raise-button');
    await expect(handBtn).toBeVisible();
    await handBtn.click(); // raise hand (this also closes the overflow)

    // Re-open overflow to lower hand
    await overflowTrigger.click();
    await expect(handBtn).toBeVisible();
    await handBtn.click(); // lower hand
  });

  test('participant grid is visible', async ({ page }) => {
    await expect(page.getByTestId('call-participant-grid')).toBeVisible();
  });
});
