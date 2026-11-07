/**
 * PWA Service Worker registration, installation, and network status utilities.
 */

let deferredInstallPrompt: any = null;
const installListeners = new Set<(canInstall: boolean) => void>();
const updateListeners = new Set<() => void>();
let waitingWorker: ServiceWorker | null = null;

export function registerServiceWorker() {
  if (typeof window === 'undefined') {
    return;
  }

  // Capture beforeinstallprompt event for Android / Chrome / Edge
  window.addEventListener('beforeinstallprompt', (e: Event) => {
    e.preventDefault();
    deferredInstallPrompt = e;
    notifyInstallable(true);
  });

  // Reset when successfully installed
  window.addEventListener('appinstalled', () => {
    deferredInstallPrompt = null;
    notifyInstallable(false);
  });

  if (!('serviceWorker' in navigator)) {
    return;
  }

  window.addEventListener('load', () => {
    navigator.serviceWorker
      .register('/sw.js')
      .then((reg) => {
        // Detect if an updated service worker is already waiting
        if (reg.waiting) {
          waitingWorker = reg.waiting;
          notifyUpdateAvailable();
        }

        // Detect when an updated service worker is found
        reg.addEventListener('updatefound', () => {
          const newWorker = reg.installing;
          if (!newWorker) return;

          newWorker.addEventListener('statechange', () => {
            if (newWorker.state === 'installed' && navigator.serviceWorker.controller) {
              waitingWorker = newWorker;
              notifyUpdateAvailable();
            }
          });
        });
      })
      .catch((err) => {
        console.warn('PWA Service Worker registration failed:', err);
      });
  });
}

export function subscribePwaInstall(callback: (canInstall: boolean) => void): () => void {
  installListeners.add(callback);
  callback(deferredInstallPrompt !== null);
  return () => installListeners.delete(callback);
}

function notifyInstallable(canInstall: boolean) {
  for (const cb of installListeners) {
    cb(canInstall);
  }
}

export async function promptPwaInstall(): Promise<boolean> {
  if (!deferredInstallPrompt) {
    return false;
  }

  deferredInstallPrompt.prompt();
  const choice = await deferredInstallPrompt.userChoice;
  deferredInstallPrompt = null;
  notifyInstallable(false);
  return choice?.outcome === 'accepted';
}

export function subscribePwaUpdate(callback: () => void): () => void {
  updateListeners.add(callback);
  if (waitingWorker) {
    callback();
  }
  return () => updateListeners.delete(callback);
}

function notifyUpdateAvailable() {
  for (const cb of updateListeners) {
    cb();
  }
}

export function applyPwaUpdate() {
  if (waitingWorker) {
    waitingWorker.postMessage({ type: 'SKIP_WAITING' });
  }
  window.location.reload();
}

export function subscribeNetworkStatus(callback: (online: boolean) => void): () => void {
  const handleOnline = () => callback(true);
  const handleOffline = () => callback(false);

  window.addEventListener('online', handleOnline);
  window.addEventListener('offline', handleOffline);

  callback(navigator.onLine);

  return () => {
    window.removeEventListener('online', handleOnline);
    window.removeEventListener('offline', handleOffline);
  };
}
