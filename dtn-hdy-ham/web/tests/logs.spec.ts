import { test, expect } from '@playwright/test';

const mockLogs = [
  {
    id: 1,
    timestamp: '2026-11-07T12:00:00Z',
    level: 'INFO',
    target: 'dtn_hdy_ham::server',
    message: 'Starting Hardy Ham BPA node',
  },
  {
    id: 2,
    timestamp: '2026-11-07T12:00:01Z',
    level: 'WARN',
    target: 'hardy_tcpclv4',
    message: 'Peer connection attempt backoff',
  },
  {
    id: 3,
    timestamp: '2026-11-07T12:00:02Z',
    level: 'ERROR',
    target: 'hardy_ltp',
    message: 'Segment checksum mismatch',
  },
  {
    id: 4,
    timestamp: '2026-11-07T12:00:03Z',
    level: 'DEBUG',
    target: 'dtn_hdy_ham::chat',
    message: 'Stored chat message from dtn://f4jxq/chat',
  },
];

test.describe('System Log Console', () => {
  test.beforeEach(async ({ page }) => {
    // Mock configuration and initial logs
    await page.route('/api/v1/config', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          callsign: 'F4TEST',
          node_id: 'dtn://f4test/',
          storage: { in_memory: true },
          clas: [],
          routes: [],
        }),
      });
    });

    await page.route('/api/v1/services', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([]),
      });
    });

    await page.route('/api/v1/chat/conversations', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([]),
      });
    });

    await page.route('/api/v1/files/unread', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ unread_count: 0 }),
      });
    });

    await page.route('/api/v1/logs', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          logs: mockLogs,
          count: mockLogs.length,
          capacity: 500,
        }),
      });
    });

    // Mock SSE endpoint
    await page.route('/api/v1/events*', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: ': keepalive\n\n',
      });
    });
  });

  test('navigates to logs tab and displays log records', async ({ page }) => {
    await page.goto('/#/logs');

    // Check header
    await expect(page.getByRole('heading', { name: /System Log Console/i })).toBeVisible();

    // Check log count
    await expect(page.getByText('Showing 4 / 4 entries')).toBeVisible();

    // Check initial log messages are rendered
    await expect(page.getByText('Starting Hardy Ham BPA node')).toBeVisible();
    await expect(page.getByText('Segment checksum mismatch')).toBeVisible();
  });

  test('filters logs by severity level', async ({ page }) => {
    await page.goto('/#/logs');

    // Click ERROR filter
    await page.getByRole('button', { name: 'ERROR' }).click();
    await expect(page.getByText('Showing 1 / 4 entries')).toBeVisible();
    await expect(page.getByText('Segment checksum mismatch')).toBeVisible();
    await expect(page.getByText('Starting Hardy Ham BPA node')).not.toBeVisible();

    // Click WARN filter
    await page.getByRole('button', { name: 'WARN' }).click();
    await expect(page.getByText('Showing 1 / 4 entries')).toBeVisible();
    await expect(page.getByText('Peer connection attempt backoff')).toBeVisible();

    // Return to ALL
    await page.getByRole('button', { name: 'ALL' }).click();
    await expect(page.getByText('Showing 4 / 4 entries')).toBeVisible();
  });

  test('searches within log entries', async ({ page }) => {
    await page.goto('/#/logs');

    const searchInput = page.getByPlaceholder('Search in logs...');
    await searchInput.fill('checksum');

    await expect(page.getByText('Showing 1 / 4 entries')).toBeVisible();
    await expect(page.getByText('Segment checksum mismatch')).toBeVisible();
    await expect(page.getByText('Starting Hardy Ham BPA node')).not.toBeVisible();

    // Clear search
    await page.getByRole('button', { name: 'Clear search' }).click();
    await expect(page.getByText('Showing 4 / 4 entries')).toBeVisible();
  });

  test('clears local logs on clear button click', async ({ page }) => {
    await page.goto('/#/logs');

    await expect(page.getByText('Showing 4 / 4 entries')).toBeVisible();
    await page.getByRole('button', { name: '🗑️ Clear' }).click();

    await expect(page.getByText('Showing 0 / 0 entries')).toBeVisible();
    await expect(page.getByText('No log records match current filters')).toBeVisible();
  });
});
