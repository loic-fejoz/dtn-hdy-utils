import { test, expect } from '@playwright/test';

const mockStationConfig = {
  status: 'running',
  version: '0.1.0',
  callsign: 'F4JXQ',
  node_id: 'dtn://f4jxq/',
  admin_endpoints: ['dtn://f4jxq/~admin'],
  storage: {
    in_memory: false,
    sqlite_path: '/var/lib/dtn-hdy-ham/bundles.db',
  },
  grpc: {
    enabled: true,
    address: '127.0.0.1:50051',
  },
  web: {
    address: '0.0.0.0:8080',
    static_dir: './web/dist',
  },
  clas: [
    {
      name: 'tcp-f4jxq',
      type: 'tcpclv4',
      connect: '192.168.1.100:4556',
      peers: ['192.168.1.100:4556'],
    },
    {
      name: 'mercury-kiss',
      type: 'mercury',
      port: '/dev/ttyUSB0',
      baud: 115200,
    },
  ],
  routes: [
    {
      pattern: 'dtn://f4jxq/*',
      cla: 'local',
    },
    {
      pattern: 'dtn://*',
      cla: 'tcp-f4jxq',
    },
  ],
};

test.describe('Station Overview & Identity', () => {
  test.beforeEach(async ({ page }) => {
    // Mock the station config endpoint before navigating
    await page.route('/api/v1/config', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockStationConfig),
      });
    });

    // Mock active services endpoint
    await page.route('/api/v1/services', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          {
            eid: 'dtn://f4jxq/echo',
            type: 'Built-in Echo',
            registered_at: '2026-11-04T14:00:00+01:00',
          },
        ]),
      });
    });

    // Mock static routes endpoint
    await page.route('/api/v1/routes*', async (route) => {
      if (route.request().method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(mockStationConfig.routes),
        });
      } else if (route.request().method() === 'POST') {
        const postData = route.request().postDataJSON();
        await route.fulfill({
          status: 201,
          contentType: 'application/json',
          body: JSON.stringify({ status: 'created', route: postData }),
        });
      } else if (route.request().method() === 'DELETE') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ status: 'deleted' }),
        });
      }
    });

    // Mock CLA peer status endpoint
    await page.route('/api/v1/clas/status', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          {
            address: '192.168.1.100:4556',
            status: 'connected',
            last_attempt_at: null,
          },
        ]),
      });
    });

    await page.goto('/');
  });

  test('displays station identity, callsign and node EID in navbar', async ({ page }) => {
    const navbar = page.locator('header');
    await expect(navbar).toBeVisible();

    // Verify title and badge
    await expect(navbar.getByText('Hardy BPA')).toBeVisible();
    await expect(navbar.getByText('Hamradio')).toBeVisible();

    // Verify amateur radio callsign and node EID
    await expect(navbar.getByText('F4JXQ')).toBeVisible();
    await expect(navbar.getByText('dtn://f4jxq/')).toBeVisible();
  });

  test('renders station overview card, version, and parameters', async ({ page }) => {
    // Station overview title and version
    await expect(page.getByRole('heading', { name: 'Station Overview' })).toBeVisible();
    await expect(page.getByText('v0.1.0')).toBeVisible();

    // Check node parameters in overview
    await expect(page.getByText('dtn://f4jxq/~admin')).toBeVisible();
    await expect(page.getByText('/var/lib/dtn-hdy-ham/bundles.db')).toBeVisible();
  });

  test('lists active convergence layers and routes', async ({ page }) => {
    // Check CLAs are listed (exact match to avoid matching route card "via tcp-f4jxq")
    await expect(page.getByText('tcp-f4jxq', { exact: true })).toBeVisible();
    await expect(page.getByText('mercury-kiss', { exact: true })).toBeVisible();

    // Check routes are listed
    await expect(page.getByText('dtn://f4jxq/*', { exact: true })).toBeVisible();
    await expect(page.getByText('dtn://*', { exact: true })).toBeVisible();
  });

  test('triggers manual peer connection with hand button', async ({ page }) => {
    // Intercept POST /api/v1/clas/connect
    await page.route('**/api/v1/clas/connect*', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          status: 'connected',
          peer: '192.168.1.100:4556',
          message: 'Successfully connected to peer 192.168.1.100:4556',
        }),
      });
    });

    // Check hand button exists
    const connectBtn = page.locator('button:has-text("Connect")');
    await expect(connectBtn).toBeVisible();
    await expect(connectBtn).toContainText('✋');

    // Click connect
    await connectBtn.click();

    // Verify success banner appears
    await expect(page.locator('text=Successfully connected to peer 192.168.1.100:4556')).toBeVisible();
  });

  test('renders active DTN services with permanent echo service', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Active DTN Services' })).toBeVisible();
    await expect(page.getByText('1 active')).toBeVisible();
    await expect(page.getByText('dtn://f4jxq/echo')).toBeVisible();
    await expect(page.getByText('Built-in Echo')).toBeVisible();
    await expect(page.getByText('2026-11-04T14:00:00+01:00')).toBeVisible();
  });

  test('toggles raw JSON configuration viewer', async ({ page }) => {
    const toggleBtn = page.getByRole('button', { name: 'View JSON' });
    await expect(toggleBtn).toBeVisible();

    // Initially raw JSON pre block is not displayed
    await expect(page.locator('pre')).not.toBeVisible();

    // Click to open raw JSON
    await toggleBtn.click();
    await expect(page.getByRole('button', { name: 'Hide JSON' })).toBeVisible();
    await expect(page.locator('pre')).toBeVisible();
    await expect(page.locator('pre')).toContainText('"callsign": "F4JXQ"');

    // Click again to hide
    await page.getByRole('button', { name: 'Hide JSON' }).click();
    await expect(page.locator('pre')).not.toBeVisible();
    await expect(page.getByRole('button', { name: 'View JSON' })).toBeVisible();
  });

  test('toggles dark and light mode theme', async ({ page }) => {
    const html = page.locator('html');
    const themeBtn = page.getByTitle(/Switch to (dark|light) mode/);
    await expect(themeBtn).toBeVisible();

    // Determine initial state
    const isDarkInitially = await html.evaluate((el) => el.classList.contains('dark'));

    // Toggle theme
    await themeBtn.click();
    if (isDarkInitially) {
      await expect(html).not.toHaveClass(/dark/);
    } else {
      await expect(html).toHaveClass(/dark/);
    }

    // Toggle back
    await themeBtn.click();
    if (isDarkInitially) {
      await expect(html).toHaveClass(/dark/);
    } else {
      await expect(html).not.toHaveClass(/dark/);
    }
  });

  test('displays peer connection status badge and allows adding a static route', async ({ page }) => {
    // Check connected status badge is displayed
    await expect(page.getByText('connected', { exact: false })).toBeVisible();

    // Click "+ Add Route" button
    await page.getByRole('button', { name: '+ Add Route' }).click();

    // Fill in pattern
    await page.locator('input[placeholder*="dtn://f4jxq-9/*"]').fill('dtn://f4jxq-9/*');

    // Click submit "Add Route"
    await page.getByRole('button', { name: 'Add Route', exact: true }).click();

    // Check success notification appears
    await expect(page.getByText(/Route .* successfully added/)).toBeVisible();
  });

  test('displays error message and retry button when config API fails', async ({ page }) => {
    // Override route to return 500 error
    await page.route('/api/v1/config', async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Internal Server Error' }),
      });
    });

    await page.goto('/');

    await expect(page.getByText('Error loading configuration')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Retry' })).toBeVisible();
  });
});
