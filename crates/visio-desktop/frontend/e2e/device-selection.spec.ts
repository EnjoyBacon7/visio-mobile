import { test, expect } from '@playwright/test';
import { joinMockRoom } from './tauri-mock';

test.describe('Device Selection', () => {
  test.beforeEach(async ({ page }) => {
    await joinMockRoom(page);
  });

  test('audio device picker shows input and output devices', async ({
    page,
  }) => {
    // Click the mic chevron to open device picker
    const chevron = page.getByTestId('call-mic-chevron');
    await chevron.click();

    // Should show device picker with audio devices
    const picker = page.getByTestId('device-picker-audio');
    await expect(picker).toBeVisible();

    // Should show microphone options
    await expect(picker.getByText('Built-in Microphone')).toBeVisible();
    await expect(picker.getByText('External USB Mic')).toBeVisible();

    // Should show speaker options
    await expect(picker.getByText('Built-in Speakers')).toBeVisible();
    await expect(picker.getByText('Headphones')).toBeVisible();
  });

  test('can select a different microphone', async ({ page }) => {
    await page.getByTestId('call-mic-chevron').click();

    const picker = page.getByTestId('device-picker-audio');
    await expect(picker).toBeVisible();

    // Click on the External USB Mic option
    await picker.getByText('External USB Mic').click();

    // The radio should be checked
    const radio = page
      .getByTestId('device-option-input-1')
      .locator('input[type="radio"]');
    await expect(radio).toBeChecked();
  });

  test('can select a different speaker', async ({ page }) => {
    await page.getByTestId('call-mic-chevron').click();

    const picker = page.getByTestId('device-picker-audio');
    await expect(picker).toBeVisible();

    await picker.getByText('Headphones').click();

    const radio = page
      .getByTestId('device-option-output-1')
      .locator('input[type="radio"]');
    await expect(radio).toBeChecked();
  });

  test('camera device picker shows cameras', async ({ page }) => {
    const chevron = page.getByTestId('call-camera-chevron');
    await chevron.click();

    const picker = page.getByTestId('device-picker-video');
    await expect(picker).toBeVisible();

    await expect(picker.getByText('FaceTime HD Camera')).toBeVisible();
    await expect(picker.getByText('External Webcam')).toBeVisible();
  });

  test('can select a different camera', async ({ page }) => {
    await page.getByTestId('call-camera-chevron').click();

    const picker = page.getByTestId('device-picker-video');
    await expect(picker).toBeVisible();

    await picker.getByText('External Webcam').click();

    const radio = page
      .getByTestId('device-option-camera-1')
      .locator('input[type="radio"]');
    await expect(radio).toBeChecked();
  });

  test('clicking outside device picker closes it', async ({ page }) => {
    await page.getByTestId('call-mic-chevron').click();
    await expect(page.getByTestId('device-picker-audio')).toBeVisible();

    // Click somewhere outside (the participant grid area)
    await page.getByTestId('call-participant-grid').click({ force: true });

    // Picker should close
    await expect(page.getByTestId('device-picker-audio')).not.toBeVisible();
  });

  test('default device has star indicator', async ({ page }) => {
    await page.getByTestId('call-mic-chevron').click();

    const picker = page.getByTestId('device-picker-audio');
    // The default device should show a star
    const defaultOption = page.getByTestId('device-option-input-0');
    await expect(defaultOption).toContainText('\u2605');
  });

  test('opening mic picker closes camera picker', async ({ page }) => {
    // Open camera picker first
    await page.getByTestId('call-camera-chevron').click();
    await expect(page.getByTestId('device-picker-video')).toBeVisible();

    // Now open mic picker
    await page.getByTestId('call-mic-chevron').click();
    await expect(page.getByTestId('device-picker-audio')).toBeVisible();
    await expect(page.getByTestId('device-picker-video')).not.toBeVisible();
  });
});
