import { test, expect } from '@playwright/test';

const mockStationConfig = {
  status: 'running',
  version: '0.1.0',
  callsign: 'F4JXQ',
  node_id: 'dtn://f4jxq/',
  admin_endpoints: ['dtn://f4jxq/~admin'],
  storage: { in_memory: false, sqlite_path: '/tmp/bundles.db' },
  grpc: { enabled: true, address: '127.0.0.1:50051' },
  web: { address: '0.0.0.0:8080', static_dir: './web/dist' },
  clas: [],
  routes: [],
};

const mockFilesList = [
  {
    id: '[dtn://f4abc/, 1000, 0]',
    source_eid: 'dtn://f4abc/files',
    destination_eid: 'dtn://f4jxq/files',
    creation_timestamp: 1000,
    lifetime_secs: 3600,
    expiry_timestamp: 1730764800,
    size: 256,
    payload_path: '/tmp/files/bundle_1000.bin',
    mime_type: 'text/markdown',
    is_sticky: false,
    is_read: false,
    created_at: '2026-11-04T12:00:00Z',
  },
  {
    id: '[dtn://f4def/, 2000, 0]',
    source_eid: 'dtn://f4def/files',
    destination_eid: 'dtn://f4jxq/files',
    creation_timestamp: 2000,
    lifetime_secs: 86400,
    expiry_timestamp: 1730851200,
    size: 1024,
    payload_path: '/tmp/files/bundle_2000.bin',
    mime_type: 'image/png',
    is_sticky: true,
    is_read: true,
    created_at: '2026-11-04T12:30:00Z',
  },
];

const mockDetailMarkdown = {
  bundle: {
    id: '[dtn://f4abc/, 1000, 0]',
    source_eid: 'dtn://f4abc/files',
    destination_eid: 'dtn://f4jxq/files',
    creation_timestamp: 1000,
    lifetime_secs: 3600,
    expiry_timestamp: 1730764800,
    size: 256,
    payload_path: '/tmp/files/bundle_1000.bin',
    mime_type: 'text/markdown',
    is_sticky: false,
    is_read: true,
    created_at: '2026-11-04T12:00:00Z',
  },
  payload_preview:
    '# Hamradio Weather Report 📡🛰️\n\nQTH Paris sunny 21°C.\n\n![Antenna](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==)\n\n73 de F4ABC',
  file_missing: false,
};

const mockDetailMissingFile = {
  bundle: {
    id: '[dtn://f4missing/, 3000, 0]',
    source_eid: 'dtn://f4missing/files',
    destination_eid: 'dtn://f4jxq/files',
    creation_timestamp: 3000,
    lifetime_secs: 3600,
    expiry_timestamp: 1730764800,
    size: 500,
    payload_path: '/tmp/files/ghost.bin',
    mime_type: 'application/octet-stream',
    is_sticky: false,
    is_read: true,
    created_at: '2026-11-04T13:00:00Z',
  },
  payload_preview: null,
  file_missing: true,
};

test.describe('Files Internal DTN Service (BUNDLE_LIST & Detail)', () => {
  test.beforeEach(async ({ page }) => {
    // Config route
    await page.route('/api/v1/config', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockStationConfig),
      });
    });

    // Services route
    await page.route('/api/v1/services', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          {
            eid: 'dtn://f4jxq/files',
            type: 'Built-in Files',
            registered_at: '2026-11-04T14:00:00+01:00',
          },
        ]),
      });
    });

    // Mock SSE events
    await page.route('/api/v1/events', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: 'event: connected\ndata: {}\n\n',
      });
    });

    // Files routes
    await page.route(/\/api\/v1\/files(\/.*)?$/, async (route) => {
      const url = route.request().url();
      const method = route.request().method();

      // Unread count
      if (url.endsWith('/api/v1/files/unread')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ unread_count: 1 }),
        });
        return;
      }

      // List endpoint (exact /api/v1/files)
      if (url.endsWith('/api/v1/files')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(mockFilesList),
        });
        return;
      }

      // Sticky toggle endpoint
      if (url.includes('/sticky')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ status: 'ok', is_sticky: true }),
        });
        return;
      }

      // Delete endpoint
      if (method === 'DELETE') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ status: 'deleted' }),
        });
        return;
      }

      // Missing file test case
      if (url.includes('f4missing') || url.includes('3000')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(mockDetailMissingFile),
        });
        return;
      }

      // Default bundle detail
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockDetailMarkdown),
      });
    });
  });

  test('displays Files tab with unread count badge in Navbar', async ({ page }) => {
    await page.goto('/');
    const filesTab = page.locator('header nav button:has-text("Files")');
    await expect(filesTab).toBeVisible();
    await expect(filesTab.locator('span:has-text("1")')).toBeVisible();
  });

  test('navigates to Files tab and lists file bundles with sticky & new badges', async ({ page }) => {
    await page.goto('/');
    await page.locator('header nav button:has-text("Files")').click();

    // Verify URL hash updated
    await expect(page).toHaveURL(/.*#\/files/);

    // Verify header and endpoint with actual station node ID
    await expect(page.locator('h1:has-text("File Bundles")')).toBeVisible();
    await expect(page.locator('code', { hasText: 'dtn://f4jxq/files' })).toBeVisible();

    // Verify items in list
    await expect(page.locator('text=dtn://f4abc/files')).toBeVisible();
    await expect(page.locator('span:has-text("NEW")')).toBeVisible();

    await expect(page.locator('text=dtn://f4def/files')).toBeVisible();
    await expect(page.locator('span:has-text("📌 STICKY")')).toBeVisible();
  });

  test('filters file bundles with search query', async ({ page }) => {
    await page.goto('/#/files');

    const searchInput = page.locator('input[placeholder*="Search by source EID"]');
    await searchInput.fill('f4def');

    // Only f4def should be shown
    await expect(page.locator('text=dtn://f4def/files')).toBeVisible();
    await expect(page.locator('text=dtn://f4abc/files')).not.toBeVisible();

    // Clear search
    await searchInput.fill('');
    await expect(page.locator('text=dtn://f4abc/files')).toBeVisible();
  });

  test('opens bundle detail (Level 2), renders Markdown with emoji & embedded Data-URI image', async ({ page }) => {
    await page.goto('/#/files');

    // Click on first bundle
    await page.locator('text=dtn://f4abc/files').click();

    // Verify URL hash updated to detail
    await expect(page).toHaveURL(/.*#\/files\/.+/);

    // Verify Level 2 headers
    await expect(page.locator('button:has-text("Back to Files List")')).toBeVisible();
    await expect(page.locator('span:has-text("Bundle Headers")')).toBeVisible();
    await expect(page.locator('text=dtn://f4abc/files')).toBeVisible();
    await expect(page.locator('text=dtn://f4jxq/files')).toBeVisible();

    // Verify Markdown rendered content
    await expect(page.locator('h1:has-text("Hamradio Weather Report 📡🛰️")')).toBeVisible();
    await expect(page.locator('text=QTH Paris sunny 21°C.')).toBeVisible();
    await expect(page.locator('text=73 de F4ABC')).toBeVisible();

    // Verify embedded Data-URI image is present in rendered HTML
    const renderedImg = page.locator('div.prose img');
    await expect(renderedImg).toBeVisible();
    await expect(renderedImg).toHaveAttribute('alt', 'Antenna');
    const src = await renderedImg.getAttribute('src');
    expect(src).toContain('data:image/png;base64,');

    // Test Back button restores list and hash
    await page.locator('button:has-text("Back to Files List")').click();
    await expect(page).toHaveURL(/.*#\/files/);
    await expect(page.locator('h1:has-text("File Bundles")')).toBeVisible();
  });

  test('toggles sticky status on a bundle in detail view', async ({ page }) => {
    await page.goto('/#/files/%5Bdtn%3A%2F%2Ff4abc%2F%2C%201000%2C%200%5D');

    // Click Sticky toggle button
    const stickyBtn = page.locator('button:has-text("Make Sticky")');
    await expect(stickyBtn).toBeVisible();
    await stickyBtn.click();

    // Should now show protected sticky state
    await expect(page.locator('button:has-text("Sticky (Protected)")')).toBeVisible();
  });

  test('clicking Reply navigates to Send tab with pre-filled destination', async ({ page }) => {
    await page.goto('/#/files/%5Bdtn%3A%2F%2Ff4abc%2F%2C%201000%2C%200%5D');

    // Click Reply button
    await page.locator('button:has-text("Reply")').click();

    // Check we navigated to Send Bundle view with destination pre-filled
    await expect(page).toHaveURL(/.*#\/send\?dest=dtn%3A%2F%2Ff4abc%2Ffiles/);
    await expect(page.locator('h2:has-text("Generic Bundle Dispatcher")')).toBeVisible();
    const destInput = page.locator('input[placeholder*="dtn://statb/incoming"]');
    await expect(destInput).toHaveValue('dtn://f4abc/files');
  });

  test('displays degraded warning when payload file is missing on storage', async ({ page }) => {
    await page.goto('/#/files/%5Bdtn%3A%2F%2Ff4missing%2F%2C%203000%2C%200%5D');

    // Warning banner should be rendered
    await expect(page.locator('text=The physical payload file is missing from station disk storage')).toBeVisible();
  });

  test('deletes bundle from detail view and navigates back to list', async ({ page }) => {
    // Handle confirmation prompt
    page.on('dialog', (dialog) => dialog.accept());

    await page.goto('/#/files/%5Bdtn%3A%2F%2Ff4abc%2F%2C%201000%2C%200%5D');

    // Click Delete button
    await page.locator('button:has-text("Delete")').click();

    // Should return to list view
    await expect(page).toHaveURL(/.*#\/files/);
    await expect(page.locator('h1:has-text("File Bundles")')).toBeVisible();
  });
});
