import { test, expect } from '@playwright/test';

const mockBundles = [
  {
    id: '[dtn://f4jxq/, 1000, 0]',
    source: 'dtn://f4jxq/echo',
    destination: 'dtn://remote/inbox',
    creation_time: 1000,
    created_at: '2026-11-04T12:00:00Z',
    lifetime_secs: 3600,
    size: 245,
    status: 'New',
  },
  {
    id: '[dtn://remote/sensor, 1001, 0]',
    source: 'dtn://remote/sensor',
    destination: 'dtn://f4jxq/telemetry',
    creation_time: 1001,
    created_at: '2026-11-04T12:05:00Z',
    lifetime_secs: 86400,
    size: 512,
    status: 'Waiting',
  },
];

const mockDetailText = {
  id: '[dtn://f4jxq/, 1000, 0]',
  source: 'dtn://f4jxq/echo',
  destination: 'dtn://remote/inbox',
  creation_time: 1000,
  created_at: '2026-11-04T12:00:00Z',
  lifetime_secs: 3600,
  size: 245,
  status: 'New',
  mime_type: 'text/plain',
  is_binary: false,
  payload_size: 19,
  storage_name: 'payload_001.bin',
  payload_preview: 'Hello Hamradio DTN!',
  hex_dump: null,
};

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

test.describe('Bundle Storage Inspector & Multi-Format Viewer', () => {
  test.beforeEach(async ({ page }) => {
    // Mock station config
    await page.route('/api/v1/config', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockStationConfig),
      });
    });

    // Mock active services
    await page.route('/api/v1/services', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([]),
      });
    });

    // Mock bundle list and bundle details/delete (using wildcard for query params)
    await page.route('**/api/v1/bundles*', async (route) => {
      const url = new URL(route.request().url());
      const id = url.searchParams.get('id');

      if (route.request().method() === 'DELETE') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ status: 'deleted', bundle_id: id }),
        });
        return;
      }

      if (id) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(mockDetailText),
        });
        return;
      }

      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockBundles),
      });
    });

    // Navigate to homepage
    await page.goto('/');
  });

  test('displays correct navbar tabs order: Station -> Chat -> Files -> SenML -> Send Bundle', async ({ page }) => {
    const navButtons = page.locator('header nav button');
    await expect(navButtons.nth(0)).toContainText('Station');
    await expect(navButtons.nth(1)).toContainText('Chat');
    await expect(navButtons.nth(2)).toContainText('Files');
    await expect(navButtons.nth(3)).toContainText('SenML');
    await expect(navButtons.nth(4)).toContainText('Send Bundle');
  });

  test('navigates to Bundles tab and renders bundle storage table', async ({ page }) => {
    // Click Station then Bundles sub-tab
    await page.locator('header nav button:has-text("Station")').click();
    await page.locator('button:has-text("Stockage & Paquets")').click();

    // Check title
    await expect(page.locator('h1')).toContainText('Bundle Storage Inspector');

    // Check table headers
    await expect(page.locator('table th:has-text("Status")')).toBeVisible();
    await expect(page.locator('table th:has-text("Bundle ID")')).toBeVisible();
    await expect(page.locator('table th:has-text("Source")')).toBeVisible();
    await expect(page.locator('table th:has-text("Destination")')).toBeVisible();

    // Verify both mocked bundles appear
    await expect(page.locator('text=dtn://remote/inbox')).toBeVisible();
    await expect(page.locator('text=dtn://f4jxq/telemetry')).toBeVisible();
  });

  test('filters bundles using search input and status selector', async ({ page }) => {
    await page.goto('/#/station/bundles');

    const searchInput = page.locator('input[placeholder*="Search by ID"]');
    await searchInput.fill('sensor');

    // Should only show matching sensor bundle
    await expect(page.locator('td:has-text("dtn://remote/sensor")').first()).toBeVisible();
    await expect(page.locator('text=dtn://remote/inbox')).not.toBeVisible();

    // Clear search
    await searchInput.fill('');
    await expect(page.locator('text=dtn://remote/inbox')).toBeVisible();
  });

  test('opens inspector modal, views details and switches to hex dump', async ({ page }) => {
    await page.goto('/#/station/bundles');

    // Click "Inspect" on the first bundle
    const inspectBtn = page.locator('button:has-text("🔍 Inspect")').first();
    await inspectBtn.click();

    // Modal should be visible
    await expect(page.locator('h3:has-text("Bundle Inspector")')).toBeVisible();
    await expect(page.locator('text=text/plain')).toBeVisible();
    await expect(page.locator('text=Hello Hamradio DTN!')).toBeVisible();

    // Switch view mode to Hex Dump
    await page.locator('button:has-text("Hex Dump")').click();
    await expect(page.locator('text=Offset (h)')).toBeVisible();

    // Close modal
    await page.locator('button:has-text("✕")').click();
    await expect(page.locator('h3:has-text("Bundle Inspector")')).not.toBeVisible();
  });

  test('triggers bundle delete flow with confirmation modal', async ({ page }) => {
    await page.goto('/#/station/bundles');

    // Click Delete on first bundle
    const deleteBtn = page.locator('button:has-text("🗑️")').first();
    await deleteBtn.click();

    // Confirmation modal should appear
    await expect(page.locator('h3:has-text("Delete Bundle?")')).toBeVisible();
    await expect(page.locator('text=permanently delete')).toBeVisible();

    // Confirm deletion
    await page.locator('button:has-text("Confirm Delete")').click();

    // Success notification should be displayed
    await expect(page.locator('text=deleted successfully')).toBeVisible();
  });
});
