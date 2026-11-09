import { h } from 'preact';
import { useState, useEffect, useRef } from 'preact/hooks';
import { Navbar, NavTabId } from './components/Navbar';
import { ConfigView, StationConfig, ActiveService, StationSubTab } from './components/ConfigView';
import { SendBundleView } from './components/SendBundleView';
import { ChatView, ChatMessage } from './components/ChatView';
import { FilesView } from './components/FilesView';
import { SenmlView } from './components/SenmlView';
import { LogRecord } from './components/LogsView';
import { playChime } from './utils/audio';
import {
  getNotificationPermission,
  requestNotificationPermission,
  isNotificationEnabled,
  setNotificationEnabled as persistNotificationEnabled,
  sendWebNotification,
  NotificationPermissionState,
} from './utils/notifications';
import {
  subscribePwaInstall,
  promptPwaInstall,
  subscribePwaUpdate,
  applyPwaUpdate,
  subscribeNetworkStatus,
} from './utils/pwa';

function parseHash(hash: string): {
  tab: NavTabId;
  stationSubTab?: StationSubTab;
  fileId: string | null;
  destination: string | null;
} {
  const clean = hash.replace(/^#\/?/, '');
  const [pathPart, queryPart] = clean.split('?');
  const segments = pathPart.split('/').filter(Boolean);
  const head = segments[0] || 'station';

  let destination: string | null = null;
  if (queryPart) {
    const params = new URLSearchParams(queryPart);
    destination = params.get('to') || params.get('dest');
  }

  if (head === 'chat') {
    return { tab: 'chat', fileId: null, destination };
  } else if (head === 'files') {
    const fileId = segments[1] ? decodeURIComponent(segments.slice(1).join('/')) : null;
    return { tab: 'files', fileId, destination };
  } else if (head === 'senml') {
    return { tab: 'senml', fileId: null, destination };
  } else if (head === 'send') {
    return { tab: 'send', fileId: null, destination };
  } else if (head === 'bundles') {
    return { tab: 'station', stationSubTab: 'bundles', fileId: null, destination };
  } else if (head === 'logs') {
    return { tab: 'station', stationSubTab: 'logs', fileId: null, destination };
  } else if (head === 'station' || head === 'config') {
    const validSubTabs: StationSubTab[] = ['radio', 'routes', 'security', 'bundles', 'logs', 'advanced'];
    const sub = segments[1] as StationSubTab;
    const stationSubTab: StationSubTab = validSubTabs.includes(sub) ? sub : 'radio';
    return { tab: 'station', stationSubTab, fileId: null, destination };
  }
  return { tab: 'station', stationSubTab: 'radio', fileId: null, destination };
}

export function App() {
  const initialNav = parseHash(window.location.hash);
  const [activeTab, setActiveTab] = useState<NavTabId>(initialNav.tab);
  const [stationSubTab, setStationSubTab] = useState<StationSubTab>(initialNav.stationSubTab || 'radio');
  const [selectedFileId, setSelectedFileId] = useState<string | null>(initialNav.fileId);
  const [sendDestination, setSendDestination] = useState<string | null>(initialNav.destination);

  const [darkMode, setDarkMode] = useState<boolean>(() => {
    return (
      localStorage.getItem('theme') === 'dark' ||
      (!('theme' in localStorage) &&
        window.matchMedia('(prefers-color-scheme: dark)').matches)
    );
  });

  const [soundEnabled, setSoundEnabled] = useState<boolean>(() => {
    return localStorage.getItem('dtn_sound_notifications') !== 'false';
  });
  const soundEnabledRef = useRef(soundEnabled);
  soundEnabledRef.current = soundEnabled;

  const [config, setConfig] = useState<StationConfig | null>(null);
  const [services, setServices] = useState<ActiveService[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [bundlesKey, setBundlesKey] = useState(0);
  const [unreadChatTotal, setUnreadChatTotal] = useState(0);
  const [unreadFilesTotal, setUnreadFilesTotal] = useState(0);
  const [lastChatMessage, setLastChatMessage] = useState<ChatMessage | null>(null);

  const [sseStatus, setSseStatus] = useState<'connected' | 'disconnected' | 'connecting'>('connecting');
  const [incomingLog, setIncomingLog] = useState<LogRecord | null>(null);

  // PWA & Network status states
  const [canInstall, setCanInstall] = useState(false);
  const [updateAvailable, setUpdateAvailable] = useState(false);
  const [isOnline, setIsOnline] = useState(true);

  // Web Notification states
  const [notifPermission, setNotifPermission] = useState<NotificationPermissionState>(() =>
    getNotificationPermission()
  );
  const [notifEnabled, setNotifEnabled] = useState<boolean>(() => isNotificationEnabled());
  const notifEnabledRef = useRef(notifEnabled);
  notifEnabledRef.current = notifEnabled;

  const handleToggleNotification = async () => {
    if (notifPermission === 'default') {
      const perm = await requestNotificationPermission();
      setNotifPermission(perm);
      if (perm === 'granted') {
        setNotifEnabled(true);
        persistNotificationEnabled(true);
        sendWebNotification('Hardy Ham DTN', {
          body: 'Notifications activées pour les alertes et messages de chat.',
        });
      }
    } else if (notifPermission === 'granted') {
      const next = !notifEnabled;
      setNotifEnabled(next);
      persistNotificationEnabled(next);
    }
  };

  useEffect(() => {
    const unsubInstall = subscribePwaInstall(setCanInstall);
    const unsubUpdate = subscribePwaUpdate(() => setUpdateAvailable(true));
    const unsubNet = subscribeNetworkStatus(setIsOnline);

    return () => {
      unsubInstall();
      unsubUpdate();
      unsubNet();
    };
  }, []);

  const handleInstallClick = async () => {
    const installed = await promptPwaInstall();
    if (installed) {
      setCanInstall(false);
    }
  };

  // Sync state with browser hash navigation
  useEffect(() => {
    const handleHashChange = () => {
      const nav = parseHash(window.location.hash);
      setActiveTab(nav.tab);
      if (nav.stationSubTab) {
        setStationSubTab(nav.stationSubTab);
      }
      setSelectedFileId(nav.fileId);
      if (nav.destination) {
        setSendDestination(nav.destination);
      }
    };

    window.addEventListener('hashchange', handleHashChange);
    return () => window.removeEventListener('hashchange', handleHashChange);
  }, []);

  const navigateToTab = (tab: NavTabId) => {
    setActiveTab(tab);
    if (tab === 'files') {
      window.location.hash = '#/files';
      setSelectedFileId(null);
    } else if (tab === 'station') {
      window.location.hash = stationSubTab ? `#/station/${stationSubTab}` : '#/station/radio';
    } else {
      window.location.hash = '#/' + tab;
    }
  };

  const handleStationSubTabChange = (subTab: StationSubTab) => {
    setStationSubTab(subTab);
    window.location.hash = `#/station/${subTab}`;
  };

  const handleSelectFile = (id: string | null) => {
    setSelectedFileId(id);
    if (id) {
      window.location.hash = `#/files/${encodeURIComponent(id)}`;
    } else {
      window.location.hash = '#/files';
    }
  };

  const handleReplyFile = (sourceEid: string) => {
    setSendDestination(sourceEid);
    window.location.hash = `#/send?dest=${encodeURIComponent(sourceEid)}`;
  };

  const handleToggleSound = () => {
    const next = !soundEnabled;
    setSoundEnabled(next);
    localStorage.setItem('dtn_sound_notifications', next ? 'true' : 'false');
    if (next) {
      playChime();
    }
  };

  // Dark mode effect
  useEffect(() => {
    if (darkMode) {
      document.documentElement.classList.add('dark');
      localStorage.setItem('theme', 'dark');
    } else {
      document.documentElement.classList.remove('dark');
      localStorage.setItem('theme', 'light');
    }
  }, [darkMode]);

  // Initial data loading
  const fetchConfig = async () => {
    try {
      const res = await fetch('/api/v1/config');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      setConfig(data);
    } catch (err: any) {
      setError(err.message || 'Failed to fetch config');
    }
  };

  const fetchServices = async () => {
    try {
      const res = await fetch('/api/v1/services');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      setServices(data);
    } catch (err) {
      console.warn('Failed to fetch services', err);
    }
  };

  const fetchUnreadChat = async () => {
    try {
      const res = await fetch('/api/v1/chat/conversations');
      if (!res.ok) return;
      const convs = await res.json();
      if (Array.isArray(convs)) {
        const total = convs.reduce((sum: number, c: any) => sum + (c.unread_count || 0), 0);
        setUnreadChatTotal(total);
      }
    } catch (err) {
      console.warn('Failed to fetch unread chat count', err);
    }
  };

  const fetchUnreadFiles = async () => {
    try {
      const res = await fetch('/api/v1/files/unread');
      if (!res.ok) return;
      const data = await res.json();
      const count = data.unread_count ?? data.unread ?? 0;
      setUnreadFilesTotal(count);
    } catch (err) {
      console.warn('Failed to fetch unread files count', err);
    }
  };

  const refreshAll = async () => {
    setLoading(true);
    await Promise.all([fetchConfig(), fetchServices(), fetchUnreadChat(), fetchUnreadFiles()]);
    setLoading(false);
  };

  useEffect(() => {
    refreshAll();
  }, []);

  // Setup Server-Sent Events (SSE) listener for reactive updates
  useEffect(() => {
    let es: EventSource | null = null;
    try {
      const sseUrl = activeTab === 'logs' ? '/api/v1/events?logs=true' : '/api/v1/events';
      setSseStatus('connecting');
      es = new EventSource(sseUrl);

      es.onopen = () => {
        setSseStatus('connected');
      };

      es.onerror = () => {
        setSseStatus('disconnected');
      };

      es.addEventListener('services_update', (e) => {
        try {
          const updated: ActiveService[] = JSON.parse(e.data);
          setServices(updated);
        } catch (err) {
          console.error('Failed to parse services_update event', err);
        }
      });

      es.addEventListener('bundles_update', () => {
        setBundlesKey((k) => k + 1);
        fetchUnreadFiles();
      });

      es.addEventListener('chat_message', (e) => {
        try {
          const msg: ChatMessage = JSON.parse(e.data);
          setLastChatMessage(msg);
          fetchUnreadChat();
          if (soundEnabledRef.current) {
            playChime();
          }
          if (document.visibilityState === 'hidden' && notifEnabledRef.current) {
            sendWebNotification(
              `[DTN Chat] ${msg.source}`,
              {
                body: msg.text || 'Nouveau message reçu',
                tag: `dtn-chat-${msg.source}`,
              },
              () => navigateToTab('chat')
            );
          }
        } catch (err) {
          console.error('Failed to parse chat_message event', err);
        }
      });

      es.addEventListener('file_received', () => {
        setBundlesKey((k) => k + 1);
        fetchUnreadFiles();
        if (soundEnabledRef.current) {
          playChime();
        }
      });

      es.addEventListener('cla_state_changed', () => {
        fetchConfig();
      });

      es.addEventListener('contact_window', () => {
        fetchConfig();
      });

      es.addEventListener('log_entry', (e) => {
        try {
          const log: LogRecord = JSON.parse(e.data);
          setIncomingLog(log);
        } catch (err) {
          console.error('Failed to parse log_entry event', err);
        }
      });
    } catch (err) {
      console.warn('Failed to initialize EventSource SSE connection', err);
      setSseStatus('disconnected');
    }

    return () => {
      if (es) {
        es.close();
      }
    };
  }, [activeTab]);

  // Collect known peer candidates from config safely
  const peerSuggestions: string[] = Array.from(
    new Set([
      ...(config?.clas || []).flatMap((c: any) => {
        const list: string[] = [];
        if (Array.isArray(c.peers)) {
          for (const p of c.peers) {
            if (typeof p === 'string') list.push(p);
            else if (p && typeof p === 'object') {
              if (p.callsign) list.push(p.callsign);
              if (p.node_id) list.push(p.node_id);
            }
          }
        }
        if (typeof c.connect === 'string') {
          list.push(c.connect);
        } else if (Array.isArray(c.connect)) {
          list.push(...c.connect);
        }
        return list;
      }),
      ...(config?.routes || []).flatMap((r: any) => {
        const list: string[] = [];
        if (r.next_hop) list.push(r.next_hop);
        if (r['next-hop']) list.push(r['next-hop']);
        if (typeof r.pattern === 'string' && r.pattern.startsWith('dtn://') && !r.pattern.includes('**')) {
          const clean = r.pattern.replace('dtn://', '').replace('/*', '').replace('*', '');
          if (clean && !clean.includes('/')) list.push(clean);
        }
        return list;
      }),
    ])
  ).filter(Boolean);

  return (
    <div class="min-h-screen flex flex-col bg-slate-50 dark:bg-slate-900 transition-colors">
      <a
        href="#main-content"
        class="sr-only focus:not-sr-only focus:fixed focus:top-3 focus:left-3 focus:z-50 focus:px-4 focus:py-2 focus:bg-emerald-600 focus:text-white focus:rounded-lg focus:shadow-lg focus:outline-none focus:ring-2 focus:ring-white text-sm font-semibold"
      >
        Skip to main content
      </a>

      {/* Offline Status Warning Banner */}
      {!isOnline && (
        <div class="bg-amber-500 text-slate-950 px-4 py-2 text-xs font-semibold text-center flex items-center justify-center space-x-2 shadow-sm">
          <span>📡</span>
          <span>Mode hors-ligne actif — Consultation locale des messages et fichiers mis en cache.</span>
        </div>
      )}

      {/* PWA Update Banner */}
      {updateAvailable && (
        <div class="bg-emerald-600 text-white px-4 py-2 text-xs font-semibold flex items-center justify-between shadow-md">
          <div class="flex items-center space-x-2">
            <span>⚡</span>
            <span>Une nouvelle version de l'application est disponible.</span>
          </div>
          <button
            onClick={applyPwaUpdate}
            class="px-3 py-1 bg-white text-emerald-800 rounded text-xs font-bold hover:bg-emerald-50 transition-colors shadow-xs"
          >
            Mettre à jour
          </button>
        </div>
      )}

      <Navbar
        activeTab={activeTab}
        onTabChange={navigateToTab}
        darkMode={darkMode}
        onToggleDarkMode={() => setDarkMode(!darkMode)}
        soundEnabled={soundEnabled}
        onToggleSound={handleToggleSound}
        notificationPermission={notifPermission}
        notificationEnabled={notifEnabled}
        onToggleNotification={handleToggleNotification}
        sseStatus={sseStatus}
        stationCallsign={config?.callsign || undefined}
        nodeId={config?.node_id}
        unreadChatCount={unreadChatTotal}
        unreadFilesCount={unreadFilesTotal}
        canInstall={canInstall}
        onInstall={handleInstallClick}
      />

      {/* Main content with bottom padding for mobile navigation bar */}
      <main id="main-content" class="flex-1 pb-20 md:pb-6" tabIndex={-1}>
        {activeTab === 'station' ? (
          <ConfigView
            config={config}
            services={services}
            loading={loading}
            error={error}
            onRefresh={refreshAll}
            activeSubTab={stationSubTab}
            onSubTabChange={handleStationSubTabChange}
            bundlesKey={bundlesKey}
            onSendReply={(dest) => {
              setSendDestination(dest);
              navigateToTab('send');
            }}
            incomingLog={incomingLog}
            sseStatus={sseStatus}
          />
        ) : activeTab === 'chat' ? (
          <ChatView
            stationCallsign={config?.callsign || undefined}
            stationNodeId={config?.node_id}
            peerSuggestions={peerSuggestions}
            lastReceivedChatMessage={lastChatMessage}
            onUnreadTotalChange={setUnreadChatTotal}
          />
        ) : activeTab === 'files' ? (
          <FilesView
            stationNodeId={config?.node_id}
            stationCallsign={config?.callsign || undefined}
            selectedBundleId={selectedFileId}
            onSelectBundle={handleSelectFile}
            onReply={handleReplyFile}
            onUnreadChange={setUnreadFilesTotal}
          />
        ) : activeTab === 'senml' ? (
          <SenmlView />
        ) : (
          <SendBundleView
            stationNodeId={config?.node_id}
            stationCallsign={config?.callsign || undefined}
            defaultDestination={sendDestination || undefined}
          />
        )}
      </main>

      <footer class="hidden md:block py-4 border-t border-slate-200 dark:border-slate-800 text-center text-xs text-slate-400 font-mono">
        DTN Hardy Ham BPA · Amateur Radio DTN BPv7 (RFC 9171) Web Console
      </footer>
    </div>
  );
}
