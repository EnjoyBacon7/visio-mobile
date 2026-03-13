import { test, expect } from '@playwright/test';
import { joinMockRoom } from './tauri-mock';

test.describe('Chat', () => {
  test.beforeEach(async ({ page }) => {
    await joinMockRoom(page);
  });

  test('chat button opens chat sidebar', async ({ page }) => {
    await page.getByTestId('call-chat-button').click();
    await expect(page.getByTestId('call-chat-sidebar')).toBeVisible();
  });

  test('chat shows empty state when no messages', async ({ page }) => {
    await page.getByTestId('call-chat-button').click();
    await expect(page.getByTestId('chat-empty')).toBeVisible();
  });

  test('can type and send a chat message', async ({ page }) => {
    await page.getByTestId('call-chat-button').click();

    const input = page.getByTestId('chat-message-input');
    await input.fill('Hello from Playwright');

    await page.getByTestId('chat-send-button').click();

    // Input should be cleared after send
    await expect(input).toHaveValue('');
  });

  test('send button is disabled when input is empty', async ({ page }) => {
    await page.getByTestId('call-chat-button').click();

    const sendBtn = page.getByTestId('chat-send-button');
    const input = page.getByTestId('chat-message-input');

    // Empty input
    await expect(input).toHaveValue('');
    // Send button should be disabled
    await expect(sendBtn).toBeDisabled();
  });

  test('send button becomes enabled when text is entered', async ({
    page,
  }) => {
    await page.getByTestId('call-chat-button').click();

    const sendBtn = page.getByTestId('chat-send-button');
    const input = page.getByTestId('chat-message-input');

    await expect(sendBtn).toBeDisabled();
    await input.fill('Hello');
    await expect(sendBtn).toBeEnabled();
  });

  test('can close chat sidebar', async ({ page }) => {
    await page.getByTestId('call-chat-button').click();
    await expect(page.getByTestId('call-chat-sidebar')).toBeVisible();

    await page.getByTestId('chat-close-button').click();
    await expect(page.getByTestId('call-chat-sidebar')).not.toBeVisible();
  });

  test('can send message with Enter key', async ({ page }) => {
    await page.getByTestId('call-chat-button').click();

    const input = page.getByTestId('chat-message-input');
    await input.fill('Hello via Enter');
    await input.press('Enter');

    // Input should be cleared after send
    await expect(input).toHaveValue('');
  });

  test('chat message list is visible', async ({ page }) => {
    await page.getByTestId('call-chat-button').click();
    await expect(page.getByTestId('chat-message-list')).toBeVisible();
  });
});

test.describe('Chat with pre-existing messages', () => {
  test('shows existing messages instead of empty state', async ({ page }) => {
    await joinMockRoom(page, {
      messages: [
        {
          id: 'msg-1',
          text: 'Hello!',
          sender_sid: 'PA_remote1',
          sender_name: 'E2E Bot',
          timestamp_ms: Date.now() - 60000,
        },
        {
          id: 'msg-2',
          text: 'Hi there!',
          sender_sid: 'PA_local',
          sender_name: 'Test User',
          timestamp_ms: Date.now() - 30000,
        },
      ],
    });

    await page.getByTestId('call-chat-button').click();

    // Should NOT show empty state
    await expect(page.getByTestId('chat-empty')).not.toBeVisible();

    // Should show messages
    await expect(page.getByTestId('chat-message-list')).toBeVisible();
    await expect(page.getByText('Hello!')).toBeVisible();
    await expect(page.getByText('Hi there!')).toBeVisible();
  });

  test('messages have bubble test ids', async ({ page }) => {
    await joinMockRoom(page, {
      messages: [
        {
          id: 'msg-1',
          text: 'First message',
          sender_sid: 'PA_remote1',
          sender_name: 'E2E Bot',
          timestamp_ms: Date.now() - 60000,
        },
        {
          id: 'msg-2',
          text: 'Second message',
          sender_sid: 'PA_local',
          sender_name: 'Test User',
          timestamp_ms: Date.now() - 30000,
        },
      ],
    });

    await page.getByTestId('call-chat-button').click();

    await expect(page.getByTestId('chat-bubble-0')).toBeVisible();
    await expect(page.getByTestId('chat-bubble-1')).toBeVisible();
  });
});
