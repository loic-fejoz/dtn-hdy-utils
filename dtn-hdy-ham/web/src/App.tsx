import { h } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import { Navbar } from './components/Navbar';
import { ConfigView, StationConfig, ActiveService } from './components/ConfigView';
import { BundlesView } from './components/BundlesView';
import { SendBundleView } from './components/SendBundleView';

export function App() {
  const [activeTab, setActiveTab] = useState<'config' | 'bundles' | 'send'>('config');
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

  const refreshAll = () => {
    fetchConfig();
    fetchServices();
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

  return (
    <div class="min-h-screen flex flex-col bg-slate-50 dark:bg-slate-900 transition-colors">
      <Navbar
        activeTab={activeTab}
        onTabChange={setActiveTab}
        darkMode={darkMode}
        onToggleDarkMode={() => setDarkMode(!darkMode)}
        stationCallsign={config?.callsign || undefined}
        nodeId={config?.node_id}
      />

      <main class="flex-1">
        {activeTab === 'config' ? (
          <ConfigView
            config={config}
            services={services}
            loading={loading}
            error={error}
            onRefresh={refreshAll}
          />
        ) : activeTab === 'bundles' ? (
          <BundlesView
            key={bundlesKey}
            onRefresh={refreshAll}
          />
        ) : (
          <SendBundleView
            stationNodeId={config?.node_id}
          />
        )}
      </main>

      <footer class="py-4 border-t border-slate-200 dark:border-slate-800 text-center text-xs text-slate-400 font-mono">
        Hardy BPA · Amateur Radio DTN BPv7 (RFC 9171) Web Console
      </footer>
    </div>
  );
}
