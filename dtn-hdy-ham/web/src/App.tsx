import { h } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import { Navbar } from './components/Navbar';
import { ConfigView, StationConfig, ActiveService } from './components/ConfigView';
import { BundlesView } from './components/BundlesView';
import { SendBundleView } from './components/SendBundleView';
import { ChatView, ChatMessage } from './components/ChatView';
import { FilesView } from './components/FilesView';

function parseHash(hash: string): {
  tab: 'config' | 'chat' | 'files' | 'bundles' | 'send';
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
  }
  return { tab: 'config', fileId: null, destination };
}

export function App() {
  const initialNav = parseHash(window.location.hash);
  const [activeTab, setActiveTab] = useState<'config' | 'chat' | 'files' | 'bundles' | 'send'>(initialNav.tab);
  const [selectedFileId, setSelectedFileId] = useState<string | null>(initialNav.fileId);
  const [sendDestination, setSendDestination] = useState<string | null>(initialNav.destination);

  const [darkMode, setDarkMode] = useState<boolean>(() => {
    return (
      localStorage.getItem('theme') === 'dark' ||
      (!('theme' in localStorage) &&
        window.matchMedia('(prefers-color-scheme: dark)').matches)
    );
  });

  const [config, setConfig] = useState<StationConfig | null>(null);
  const [services, setServices] = useState<ActiveService[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [bundlesKey, setBundlesKey] = useState(0);
  const [unreadChatTotal, setUnreadChatTotal] = useState(0);
  const [unreadFilesTotal, setUnreadFilesTotal] = useState(0);
  const [lastChatMessage, setLastChatMessage] = useState<ChatMessage | null>(null);

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

  const navigateToTab = (tab: 'config' | 'chat' | 'files' | 'bundles' | 'send') => {
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

  // Sync theme with <html> class
  useEffect(() => {
    if (darkMode) {
      document.documentElement.classList.add('dark');
      localStorage.setItem('theme', 'dark');
    } else {
      document.documentElement.classList.remove('dark');
      localStorage.setItem('theme', 'light');
    }
  }, [darkMode]);

  // Fetch configuration from API
  const fetchConfig = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch('/api/v1/config');
      if (!res.ok) {
        throw new Error(`Failed to fetch config: HTTP ${res.status}`);
      }
      const data: StationConfig = await res.json();
      setConfig(data);
    } catch (err: any) {
      setError(err.message || 'Error connecting to Hardy BPA Web API');
    } finally {
      setLoading(false);
    }
  };

  // Fetch active services snapshot
  const fetchServices = async () => {
    try {
      const res = await fetch('/api/v1/services');
      if (res.ok) {
        const data: ActiveService[] = await res.json();
        setServices(data);
      }
    } catch {
      // Retain existing services if network hiccup
    }
  };

  // Fetch unread chat messages total
  const fetchUnreadChat = async () => {
    try {
      const res = await fetch('/api/v1/chat/conversations');
      if (res.ok) {
        const convs = await res.json();
        const total = convs.reduce((acc: number, c: any) => acc + (c.unread_count || 0), 0);
        setUnreadChatTotal(total);
      }
    } catch {
      // Retain existing
    }
  };

  // Fetch unread files total
  const fetchUnreadFiles = async () => {
    try {
      const res = await fetch('/api/v1/files/unread');
      if (res.ok) {
        const data = await res.json();
        setUnreadFilesTotal(data.unread_count ?? data.unread ?? 0);
      }
    } catch {
      // Retain existing
    }
  };

  const refreshAll = () => {
    fetchConfig();
    fetchServices();
    fetchUnreadChat();
    fetchUnreadFiles();
  };

  useEffect(() => {
    refreshAll();

    // Setup Server-Sent Events (SSE) listener for reactive updates
    let es: EventSource | null = null;
    try {
      es = new EventSource('/api/v1/events');
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
        } catch (err) {
          console.error('Failed to parse chat_message event', err);
        }
      });
    } catch (err) {
      console.warn('Failed to initialize EventSource SSE connection', err);
    }

    return () => {
      if (es) {
        es.close();
      }
    };
  }, []);

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
