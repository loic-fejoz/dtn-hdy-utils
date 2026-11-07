import { h } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import { Navbar } from './components/Navbar';
import { ConfigView, StationConfig, ActiveService } from './components/ConfigView';
import { BundlesView } from './components/BundlesView';
import { SendBundleView } from './components/SendBundleView';
import { ChatView, ChatMessage } from './components/ChatView';
import { FilesView } from './components/FilesView';
import { LogsView, LogRecord } from './components/LogsView';
import { playChime } from './utils/audio';

function parseHash(hash: string): {
  tab: 'config' | 'chat' | 'files' | 'bundles' | 'send' | 'logs';
  fileId: string | null;
  destination: string | null;
} {
  const clean = hash.replace(/^#\/?/, '');
  const [pathPart, queryPart] = clean.split('?');
  const segments = pathPart.split('/').filter(Boolean);
  const head = segments[0] || 'config';

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
  } else if (head === 'bundles') {
    return { tab: 'bundles', fileId: null, destination };
  } else if (head === 'send') {
    return { tab: 'send', fileId: null, destination };
  } else if (head === 'logs') {
    return { tab: 'logs', fileId: null, destination };
  }
  return { tab: 'config', fileId: null, destination };
}

export function App() {
  const initialNav = parseHash(window.location.hash);
  const [activeTab, setActiveTab] = useState<'config' | 'chat' | 'files' | 'bundles' | 'send' | 'logs'>(initialNav.tab);
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

  // Sync state with browser hash navigation
  useEffect(() => {
    const handleHashChange = () => {
      const nav = parseHash(window.location.hash);
      setActiveTab(nav.tab);
      setSelectedFileId(nav.fileId);
      if (nav.destination) {
        setSendDestination(nav.destination);
      }
    };

    window.addEventListener('hashchange', handleHashChange);
    return () => window.removeEventListener('hashchange', handleHashChange);
  }, []);

  const navigateToTab = (tab: 'config' | 'chat' | 'files' | 'bundles' | 'send' | 'logs') => {
    setActiveTab(tab);
    if (tab === 'files') {
      window.location.hash = '#/files';
      setSelectedFileId(null);
    } else {
      window.location.hash = '#/' + tab;
    }
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
    } catch (err: any) {
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
          if (soundEnabled) {
            playChime();
          }
        } catch (err) {
          console.error('Failed to parse chat_message event', err);
        }
      });

      es.addEventListener('file_received', () => {
        setBundlesKey((k) => k + 1);
        fetchUnreadFiles();
        if (soundEnabled) {
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
  }, [activeTab, soundEnabled]);

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

      <Navbar
        activeTab={activeTab}
        onTabChange={navigateToTab}
        darkMode={darkMode}
        onToggleDarkMode={() => setDarkMode(!darkMode)}
        soundEnabled={soundEnabled}
        onToggleSound={handleToggleSound}
        sseStatus={sseStatus}
        stationCallsign={config?.callsign || undefined}
        nodeId={config?.node_id}
        unreadChatCount={unreadChatTotal}
        unreadFilesCount={unreadFilesTotal}
      />

      <main id="main-content" class="flex-1" tabIndex={-1}>
        {activeTab === 'config' ? (
          <ConfigView
            config={config}
            services={services}
            loading={loading}
            error={error}
            onRefresh={refreshAll}
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
        ) : activeTab === 'bundles' ? (
          <BundlesView
            key={bundlesKey}
            onRefresh={refreshAll}
          />
        ) : activeTab === 'logs' ? (
          <LogsView
            incomingLog={incomingLog}
            sseStatus={sseStatus}
          />
        ) : (
          <SendBundleView
            stationNodeId={config?.node_id}
            stationCallsign={config?.callsign || undefined}
            defaultDestination={sendDestination || undefined}
          />
        )}
      </main>

      <footer class="py-4 border-t border-slate-200 dark:border-slate-800 text-center text-xs text-slate-400 font-mono">
        DTN Hardy Ham BPA · Amateur Radio DTN BPv7 (RFC 9171) Web Console
      </footer>
    </div>
  );
}
