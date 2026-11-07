# ADR 19: Progressive Web App (PWA) & Field Mobile Ergonomics

## Status
Accepted

## Context
Operators deployed in field operations, emergency communication exercises (EmComm), or remote repeater huts frequently interact with `dtn-hdy-ham` using mobile devices (smartphones, tablets, and ruggedized touch laptops) over a local Wi-Fi hotspot hosted by the node (e.g. Raspberry Pi).
Previously:
1. The web interface functioned strictly as an online website inside a standard browser tab with full browser chrome (URL bar, navigation buttons), reducing precious vertical screen estate on smartphones.
2. In transient disconnection scenarios (e.g., an operator carrying a handheld tablet walking away from the Wi-Fi hotspot), the browser displayed generic network error pages instead of keeping cached conversations and files accessible.
3. Navigation tabs were placed in a single horizontal header bar designed for desktop viewports, making thumb navigation on small touchscreens awkward and prone to misclicks with gloved or wet hands.
4. Deploying companion native apps (such as Android APKs) across ad-hoc field teams requires side-loading, permissions management, and maintaining divergent codebases.

## Decision

We transform the `dtn-hdy-ham` web interface into an installable, offline-resilient **Progressive Web App (PWA)** tailored for field ergonomics:

### 1. Web Application Manifest & Standalone Display
- Added `manifest.webmanifest` defining:
  - Application Name: `"Hardy DTN Ham BPA"` (short name: `"Hardy Ham"`).
  - Display mode: `"standalone"` to launch in immersive app-like mode without browser URL bars.
  - Theme colors and background colors matching the dark/light Tailwind palette (`#059669` emerald and `#0f172a` slate).
  - Responsive high-resolution and maskable app icons with the amateur radio antenna motif 📡.
  - Viewport configured with `viewport-fit=cover` to support modern notch and island smartphone displays.

### 2. Service Worker (`sw.js`) & Caching Strategy
- **App Shell Pre-caching**:
  - Pre-caches core HTML, bundled CSS/JS assets, manifest, and icons on install.
  - Enables instant startup even if the hosting node is temporarily unreachable.
- **Data Caching & Offline Consultation (Cache-First / Network-First)**:
  - Read-only API queries (`/api/v1/config`, `/api/v1/chat/conversations`, `/api/v1/files`) use a Network-First strategy with fallback to previously cached responses when offline.
  - Real-time SSE streaming (`/api/v1/events*`) and state-mutating requests (POST, DELETE) are strictly Network-Only, bypassing the cache.
- **Cache Lifecycle & Background Updates**:
  - Service Worker automatically checks for newer deployments in the background.
  - When an update is detected, an unobtrusive banner notifies the operator (*"⚡ Nouvelle version disponible — [Mettre à jour]"*), activating `skipWaiting()` and purging stale caches upon confirmation.

### 3. In-App Installation Flow (A2HS)
- Listens for the `beforeinstallprompt` browser event.
- Displays a dedicated `"📲 Installer"` button in the navigation header when eligible.
- Provides accessible step-by-step guidance for iOS/Safari operators (*"Partager ⎋ > Sur l'écran d'accueil ➕"*).

### 4. Field Touch Ergonomics & Thumb-Friendly Navigation
- **Responsive Layout Split**:
  - **Desktop / Wide Tablet (≥ 640px)**: Retains the top horizontal navigation bar.
  - **Smartphone / Narrow Touchscreen (< 640px)**:
    - **Compact Top Header**: Displays station callsign, network/SSE status badge, audio alert toggle, notification toggle, and theme switch.
    - **Bottom Navigation Bar (Thumb Zone)**: Fixed bottom bar containing the main tabs (`Config`, `Chat`, `Files`, `Bundles`, `Send`, `Logs`) with generous touch targets (≥ 48×48 px) and padding accommodating modern home gesture bars (`safe-area-inset-bottom`) suitable for gloved operation.
- **Offline Network Status Banner**:
  - Displays a visual warning when the device loses connection with the node, clarifying that the app is in offline consultation mode.

### 5. Web Notification API for Background Message Alerts
- **Operator Background Awareness**:
  - Field operators monitoring emergency frequencies or handling radio gear may switch apps or lock the screen.
  - Integration of the standard browser `Notification` API:
    - Explicit opt-in button in navbar/settings.
    - Triggers native desktop/mobile system notifications on incoming chat messages when document visibility is hidden (`document.visibilityState === 'hidden'`).
    - Clicking the notification immediately focuses the app window and switches to the active chat thread.

## Consequences
- Operators can install the console directly from their browser with native app behavior and zero app-store friction.
- Conversations, files, and configuration remain readable during field Wi-Fi micro-interruptions.
- Mobile touch interactions are comfortable and error-free on handheld screens.
