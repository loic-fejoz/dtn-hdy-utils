import { test, expect } from '@playwright/test';

test.describe('PWA & Field Mobile Ergonomics', () => {
  test.beforeEach(async ({ page }) => {
    await page.route('/api/v1/config', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          callsign: 'F4PWA',
          node_id: 'dtn://f4pwa/',
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

    await page.route('/api/v1/events*', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: ': keepalive\n\n',
      });
    });
  });

  test('declares web manifest and PWA meta tags in HTML head', async ({ page }) => {
    await page.goto('/');

    const manifestLink = page.locator('link[rel="manifest"]');
    await expect(manifestLink).toHaveAttribute('href', '/manifest.webmanifest');

    const themeColor = page.locator('meta[name="theme-color"]');
    await expect(themeColor).toHaveAttribute('content', '#059669');

    const appleCapable = page.locator('meta[name="apple-mobile-web-app-capable"]');
    await expect(appleCapable).toHaveAttribute('content', 'yes');
  });

  test('renders mobile bottom navigation bar with >= 48px touch targets on smartphone viewport', async ({
    page,
  }) => {
    // Set viewport to mobile smartphone dimensions
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto('/');

    const mobileNav = page.locator('nav[aria-label="Mobile navigation"]');
    await expect(mobileNav).toBeVisible();

    // Verify all 5 tabs exist in mobile bottom bar
    const tabLabels = ['Station', 'Chat', 'Files', 'SenML', 'Send'];
    for (const label of tabLabels) {
      const btn = mobileNav.getByRole('button', { name: label });
      await expect(btn).toBeVisible();
      const box = await btn.boundingBox();
      expect(box?.height).toBeGreaterThanOrEqual(48);
      expect(box?.width).toBeGreaterThanOrEqual(48);
    }

    // Switch to Chat using bottom bar
    await mobileNav.getByRole('button', { name: 'Chat' }).click();
    await expect(page).toHaveURL(/#\/chat/);

    // Switch to Send using bottom bar
    await mobileNav.getByRole('button', { name: 'Send' }).click();
    await expect(page).toHaveURL(/#\/send/);
  });

  test('toggles web notifications button in navbar and requests permission', async ({ page }) => {
    // Mock Notification API in window
    await page.addInitScript(() => {
      let permissionState: NotificationPermission = 'default';
      (window as any).Notification = class MockNotification {
        static get permission() {
          return permissionState;
        }
        static requestPermission() {
          permissionState = 'granted';
          return Promise.resolve(permissionState);
        }
        constructor(public title: string, public options?: any) {}
      };
    });

    await page.goto('/');

    // Notification button initially present with 'Activer les notifications'
    const notifBtn = page.getByRole('button', { name: /Activer les notifications système/i });
    await expect(notifBtn).toBeVisible();

    // Click to request permission and enable
    await notifBtn.click();

    // Now button switches to 'Désactiver les notifications'
    const disableBtn = page.getByRole('button', { name: /Désactiver les notifications système/i });
    await expect(disableBtn).toBeVisible();
  });

  test('shows offline banner when device goes offline', async ({ page }) => {
    await page.goto('/');

    // Simulate window offline event
    await page.evaluate(() => {
      window.dispatchEvent(new Event('offline'));
    });

    await expect(
      page.getByText(/Mode hors-ligne actif/i)
    ).toBeVisible();

    // Simulate recovery to online
    await page.evaluate(() => {
      window.dispatchEvent(new Event('online'));
    });

    await expect(
      page.getByText(/Mode hors-ligne actif/i)
    ).not.toBeVisible();
  });

  test('displays install button when beforeinstallprompt fires', async ({ page }) => {
    await page.goto('/');

    // Simulate beforeinstallprompt event
    await page.evaluate(() => {
      const event = new Event('beforeinstallprompt');
      (event as any).prompt = () => {};
      (event as any).userChoice = Promise.resolve({ outcome: 'accepted' });
      window.dispatchEvent(event);
    });

    const installBtn = page.getByRole('button', { name: /Installer/i });
    await expect(installBtn).toBeVisible();
  });
});
