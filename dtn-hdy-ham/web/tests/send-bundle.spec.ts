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

test.describe('Send Bundle View', () => {
  test.beforeEach(async ({ page, context }) => {
    // Grant clipboard permissions for copy button test
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);

    // Mock station config
    await page.route('/api/v1/config', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockStationConfig),
      });
    });

    await page.goto('/');

    // Switch to Send Bundle tab
    await page.getByRole('button', { name: 'Send Bundle' }).click();
    await expect(page.getByRole('heading', { name: 'Generic Bundle Dispatcher' })).toBeVisible();
  });

  test('navigates to send bundle tab and shows default station placeholders', async ({ page }) => {
    const destInput = page.getByPlaceholder('dtn://statb/incoming or ipn:2.1');
    await expect(destInput).toBeVisible();
    await expect(destInput).toHaveValue('');

    const sourceInput = page.getByPlaceholder('dtn://f4jxq/');
    await expect(sourceInput).toBeVisible();

    const lifetimeInput = page.locator('input[type="number"]');
    await expect(lifetimeInput).toHaveValue('3600');
  });

  test('submits a text bundle and displays the returned bundle ID', async ({ page }) => {
    // Intercept POST /api/v1/bundles
    let interceptedRequestBody: any = null;
    await page.route('/api/v1/bundles', async (route) => {
      if (route.request().method() === 'POST') {
        interceptedRequestBody = JSON.parse(route.request().postData() || '{}');
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            status: 'sent',
            bundle_id: 'bundle://f4jxq/1730718000/0',
            source: 'dtn://f4jxq/',
            destination: 'dtn://f1xyz/bulletin',
            payload_size: 33,
            lifetime_secs: 3600,
          }),
        });
      }
    });

    // Fill the form
    await page.getByPlaceholder('dtn://statb/incoming or ipn:2.1').fill('dtn://f1xyz/bulletin');
    await page.getByPlaceholder('Type your message to transmit over DTN BPv7...').fill('73 de F4JXQ - CQ DTN test over VHF');

    // Click transmit
    await page.getByRole('button', { name: 'Transmit Bundle' }).click();

    // Verify request payload was formatted correctly
    expect(interceptedRequestBody).toEqual({
      destination: 'dtn://f1xyz/bulletin',
      lifetime: 3600,
      payload: '73 de F4JXQ - CQ DTN test over VHF',
    });

    // Verify success banner and returned bundle ID
    await expect(page.getByText('Bundle Successfully Dispatched')).toBeVisible();
    await expect(page.getByText('SENT')).toBeVisible();
    await expect(page.getByText('bundle://f4jxq/1730718000/0')).toBeVisible();
    await expect(page.getByText('From: dtn://f4jxq/')).toBeVisible();
    await expect(page.getByText('To: dtn://f1xyz/bulletin')).toBeVisible();
  });

  test('copies bundle ID to clipboard on button click', async ({ page }) => {
    // Mock successful submission
    await page.route('/api/v1/bundles', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          status: 'sent',
          bundle_id: 'bundle://f4jxq/1730718000/42',
          source: 'dtn://f4jxq/',
          destination: 'dtn://f1xyz/bulletin',
          payload_size: 10,
          lifetime_secs: 3600,
        }),
      });
    });

    await page.getByPlaceholder('dtn://statb/incoming or ipn:2.1').fill('dtn://f1xyz/bulletin');
    await page.getByPlaceholder('Type your message to transmit over DTN BPv7...').fill('Test copy');
    await page.getByRole('button', { name: 'Transmit Bundle' }).click();

    await expect(page.getByText('bundle://f4jxq/1730718000/42')).toBeVisible();

    const copyBtn = page.getByRole('button', { name: 'Copy Key' });
    await expect(copyBtn).toBeVisible();
    await copyBtn.click();

    // Verify button changes text to "Copied! ✓"
    await expect(page.getByText('Copied! ✓')).toBeVisible();
  });

  test('submits a binary file bundle and displays the returned bundle ID', async ({ page }) => {
    let requestWasMultipart = false;

    await page.route('/api/v1/bundles', async (route) => {
      if (route.request().method() === 'POST') {
        const contentType = route.request().headers()['content-type'] || '';
        requestWasMultipart = contentType.includes('multipart/form-data');
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            status: 'sent',
            bundle_id: 'bundle://f4jxq/1730718100/99',
            source: 'dtn://f4jxq/',
            destination: 'dtn://f1xyz/files',
            payload_size: 512,
            lifetime_secs: 7200,
          }),
        });
      }
    });

    await page.getByPlaceholder('dtn://statb/incoming or ipn:2.1').fill('dtn://f1xyz/files');
    await page.locator('input[type="number"]').fill('7200');

    // Switch to file mode
    await page.getByRole('button', { name: 'Binary / File Attachment' }).click();

    // Upload a test file
    await page.locator('#file-upload').setInputFiles({
      name: 'weather_report.dat',
      mimeType: 'application/octet-stream',
      buffer: Buffer.from('METAR LFRK 041200Z 24012KT 9999 FEW030 14/08 Q1018='),
    });

    // Check preview badge
    await expect(page.getByText('weather_report.dat')).toBeVisible();

    // Transmit
    await page.getByRole('button', { name: 'Transmit Bundle' }).click();

    // Verify multipart request and result
    expect(requestWasMultipart).toBe(true);
    await expect(page.getByText('Bundle Successfully Dispatched')).toBeVisible();
    await expect(page.getByText('bundle://f4jxq/1730718100/99')).toBeVisible();
    await expect(page.getByText('To: dtn://f1xyz/files')).toBeVisible();
  });

  test('displays validation error if message payload is empty', async ({ page }) => {
    await page.getByPlaceholder('dtn://statb/incoming or ipn:2.1').fill('dtn://statb/incoming');
    // Keep textarea empty
    await page.getByRole('button', { name: 'Transmit Bundle' }).click();

    await expect(page.getByText('Please enter a text message payload')).toBeVisible();
  });

  test('displays transmission failed banner when API returns error', async ({ page }) => {
    await page.route('/api/v1/bundles', async (route) => {
      await route.fulfill({
        status: 400,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Endpoint ID scheme not supported by routing table' }),
      });
    });

    await page.getByPlaceholder('dtn://statb/incoming or ipn:2.1').fill('bad://unknown');
    await page.getByPlaceholder('Type your message to transmit over DTN BPv7...').fill('Test failure');

    await page.getByRole('button', { name: 'Transmit Bundle' }).click();

    await expect(page.getByText('Transmission Failed')).toBeVisible();
    await expect(page.getByText('Endpoint ID scheme not supported by routing table')).toBeVisible();
  });
});
