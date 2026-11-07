/**
 * Web Notification API utilities for background operator alerts.
 */

export type NotificationPermissionState = 'granted' | 'denied' | 'default' | 'unsupported';

export function isNotificationSupported(): boolean {
  return typeof window !== 'undefined' && 'Notification' in window;
}

export function getNotificationPermission(): NotificationPermissionState {
  if (!isNotificationSupported()) {
    return 'unsupported';
  }
  return Notification.permission as NotificationPermissionState;
}

export async function requestNotificationPermission(): Promise<NotificationPermissionState> {
  if (!isNotificationSupported()) {
    return 'unsupported';
  }
  try {
    const perm = await Notification.requestPermission();
    return perm as NotificationPermissionState;
  } catch (err) {
    console.warn('Failed to request notification permission', err);
    return Notification.permission as NotificationPermissionState;
  }
}

export function isNotificationEnabled(): boolean {
  if (!isNotificationSupported()) return false;
  const stored = localStorage.getItem('dtn_web_notifications_enabled');
  return stored !== 'false' && Notification.permission === 'granted';
}

export function setNotificationEnabled(enabled: boolean) {
  localStorage.setItem('dtn_web_notifications_enabled', enabled ? 'true' : 'false');
}

export function sendWebNotification(
  title: string,
  options?: NotificationOptions,
  onClick?: () => void
): Notification | null {
  if (!isNotificationSupported() || !isNotificationEnabled()) {
    return null;
  }

  try {
    const notif = new Notification(title, {
      icon: '/icons/icon.svg',
      badge: '/icons/icon.svg',
      ...options,
    });

    if (onClick) {
      notif.onclick = (event) => {
        event.preventDefault();
        window.focus();
        onClick();
        notif.close();
      };
    }

    return notif;
  } catch (err) {
    console.warn('Failed to dispatch web notification', err);
    return null;
  }
}
