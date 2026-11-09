import { test, expect } from '@playwright/test';

const mockStationConfig = {
  status: 'running',
  version: '0.1.0',
  callsign: 'STATA',
  node_id: 'dtn://stata/',
  admin_endpoints: ['dtn://stata/~admin'],
  storage: { in_memory: true, sqlite_path: null },
  grpc: { enabled: false, address: '127.0.0.1:50051' },
  web: { address: '0.0.0.0:8080', static_dir: './web/dist' },
  clas: [],
  routes: [],
};

test.describe('BPSec Security & Authentication (RFC 9103 / RFC 9172)', () => {
  let mockPolicy = 'warn';
  let mockKeys: any[] = [
    {
      id: 'key-out-1',
      direction: 'outbound',
      pattern: 'dtn://f4jxq/*',
      label: 'F4JXQ Station Outbound',
      fingerprint: 'SHA256:4a2f8b9c...',
      created_at: '2026-11-07T12:00:00Z',
    },
    {
      id: 'key-in-1',
      direction: 'inbound',
      pattern: 'dtn://f5kuv/*',
      label: 'F5KUV Station Inbound',
      fingerprint: 'SHA256:7c3d1e2f...',
      created_at: '2026-11-07T12:30:00Z',
    },
  ];

  test.beforeEach(async ({ page }) => {
    mockPolicy = 'warn';
    mockKeys = [
      {
        id: 'key-out-1',
        direction: 'outbound',
        pattern: 'dtn://f4jxq/*',
        label: 'F4JXQ Station Outbound',
        fingerprint: 'SHA256:4a2f8b9c...',
        created_at: '2026-11-07T12:00:00Z',
      },
      {
        id: 'key-in-1',
        direction: 'inbound',
        pattern: 'dtn://f5kuv/*',
        label: 'F5KUV Station Inbound',
        fingerprint: 'SHA256:7c3d1e2f...',
        created_at: '2026-11-07T12:30:00Z',
      },
    ];

    await page.route('/api/v1/config', async (route) => {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(mockStationConfig) });
    });

    await page.route('/api/v1/services', async (route) => {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
    });

    await page.route('/api/v1/routes', async (route) => {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
    });

    await page.route('/api/v1/clas/status', async (route) => {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
    });

    await page.route('/api/v1/security/policy', async (route) => {
      if (route.request().method() === 'PUT') {
        const body = JSON.parse(route.request().postData() || '{}');
        mockPolicy = body.policy || mockPolicy;
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ policy: mockPolicy }) });
      } else {
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ policy: mockPolicy }) });
      }
    });

    await page.route('/api/v1/security/keys', async (route) => {
      if (route.request().method() === 'POST') {
        const body = JSON.parse(route.request().postData() || '{}');
        const newKey = {
          id: 'key-new-' + Date.now(),
          direction: body.direction,
          pattern: body.pattern,
          label: body.label || null,
          fingerprint: 'SHA256:abcdef01...',
          created_at: new Date().toISOString(),
        };
        mockKeys.push(newKey);
        await route.fulfill({ status: 201, contentType: 'application/json', body: JSON.stringify(newKey) });
      } else {
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(mockKeys) });
      }
    });

    await page.route(/\/api\/v1\/security\/keys\/.*/, async (route) => {
      if (route.request().method() === 'DELETE') {
        const url = route.request().url();
        const id = url.split('/').pop();
        mockKeys = mockKeys.filter((k) => k.id !== id);
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ success: true }) });
      }
    });
  });

  test('displays BPSec security card with directional key tables in Configuration', async ({ page }) => {
    await page.goto('/#/station/security');

    // Check Security section title
    await expect(page.locator('text=Sécurité & Authentification BPSec')).toBeVisible();

    // Verify Outbound key table has the outbound key
    await expect(page.locator('text=dtn://f4jxq/*')).toBeVisible();
    await expect(page.locator('text=F4JXQ Station Outbound')).toBeVisible();

    // Verify Inbound key table has the inbound key
    await expect(page.locator('text=dtn://f5kuv/*')).toBeVisible();
    await expect(page.locator('text=F5KUV Station Inbound')).toBeVisible();
  });

  test('allows changing verification policy', async ({ page }) => {
    await page.goto('/#/station/security');

    const policySelect = page.locator('select').filter({ hasText: 'Avertissement' });
    await expect(policySelect).toBeVisible();

    // Change to strict policy
    await policySelect.selectOption('strict');
    await expect(page.locator('text=mise à jour en mode "STRICT"')).toBeVisible();
  });

  test('allows adding a new outbound BPSec key with random generator', async ({ page }) => {
    await page.goto('/#/station/security');

    // Open Add Key form
    await page.click('button:has-text("+ Ajouter une clé")');

    // Click pattern preset
    await page.click('button:has-text("dtn://*/chat")');

    // Enter label
    await page.fill('input[placeholder*="Station F5KUV"]', 'Chat Global Outbound');

    // Click Random Secret Generator
    await page.click('text=Générer aléatoire');

    // Submit form
    await page.click('button:has-text("Enregistrer la clé")');

    // Verify new key is in the table
    await expect(page.locator('text=Chat Global Outbound')).toBeVisible();
    await expect(page.locator('td:has-text("dtn://*/chat")')).toBeVisible();
  });

  test('renders BPSec trust badges in Chat message bubbles', async ({ page }) => {
    const mockChatMessages = [
      {
        id: 'msg-1',
        conversation_peer: 'dtn://f4jxq/chat',
        sender_eid: 'dtn://f4jxq/chat',
        recipient_eid: 'dtn://stata/chat',
        content: 'Authentic 59 signal de F4JXQ',
        direction: 'inbound',
        status: 'received',
        is_read: true,
        timestamp: Date.now() - 60000,
        created_at: new Date(Date.now() - 60000).toISOString(),
        auth_status: 'valid',
      },
      {
        id: 'msg-2',
        conversation_peer: 'dtn://f4jxq/chat',
        sender_eid: 'dtn://f4jxq/chat',
        recipient_eid: 'dtn://stata/chat',
        content: 'Warning: tampered bundle received',
        direction: 'inbound',
        status: 'received',
        is_read: true,
        timestamp: Date.now() - 40000,
        created_at: new Date(Date.now() - 40000).toISOString(),
        auth_status: 'invalid',
      },
      {
        id: 'msg-3',
        conversation_peer: 'dtn://f4jxq/chat',
        sender_eid: 'dtn://f4jxq/chat',
        recipient_eid: 'dtn://stata/chat',
        content: 'Signed by unknown station without key',
        direction: 'inbound',
        status: 'received',
        is_read: true,
        timestamp: Date.now() - 20000,
        created_at: new Date(Date.now() - 20000).toISOString(),
        auth_status: 'missing_key',
      },
    ];

    await page.route('/api/v1/events', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: 'event: connected\ndata: {}\n\n',
      });
    });

    await page.route('/api/v1/chat/read', async (route) => {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ success: true }) });
    });

    await page.route('/api/v1/chat/conversations', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          {
            peer: 'dtn://f4jxq/chat',
            callsign: 'F4JXQ',
            last_message: 'Signed by unknown station without key',
            last_direction: 'inbound',
            last_status: 'received',
            last_timestamp: Date.now() - 20000,
            last_created_at: new Date(Date.now() - 20000).toISOString(),
            unread_count: 0,
            total_messages: 3,
          },
        ]),
      });
    });

    await page.route(/\/api\/v1\/chat\/messages\?peer=.*/, async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockChatMessages),
      });
    });

    await page.goto('/');

    // Navigate to Chat
    await page.click('button:has-text("Chat"), a:has-text("Chat")');

    // Click conversation
    await page.click('text=F4JXQ');

    // Verify all 3 trust badges render in messages
    await expect(page.locator('span[aria-label="Authentifié"]').first()).toBeVisible();
    await expect(page.locator('span[aria-label="Signature Invalide"]').first()).toBeVisible();
    await expect(page.locator('span[aria-label="Clé manquante"]').first()).toBeVisible();
  });
});
