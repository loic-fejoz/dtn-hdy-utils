import { h } from 'preact';
import { useState, useEffect } from 'preact/hooks';

export interface ActiveService {
  eid: string;
  type: string;
  registered_at: string;
}

export interface RouteItem {
  pattern: string;
  cla: string;
  'next-hop'?: string | null;
  next_hop?: string | null;
  priority?: number;
}

export interface PeerStatusInfo {
  address: string;
  status: 'connected' | 'disconnected' | 'connecting';
  last_attempt_at: string | null;
}

export interface KeyInfo {
  id: string;
  direction: 'outbound' | 'inbound';
  pattern: string;
  label?: string | null;
  fingerprint: string;
  created_at: string;
}

export type VerifyPolicy = 'warn' | 'strict' | 'ignore';

export interface StationConfig {
  status: string;
  version: string;
  callsign: string | null;
  node_id: string;
  admin_endpoints: string[];
  storage: {
    in_memory: boolean;
    sqlite_path: string | null;
  };
  grpc: {
    enabled: boolean;
    address: string;
  };
  web: {
    address: string;
    static_dir: string;
  } | null;
  clas: Array<{
    name: string;
    type: string;
    [key: string]: any;
  }>;
  routes: Array<RouteItem>;
}

interface ConfigViewProps {
  config: StationConfig | null;
  services?: ActiveService[];
  loading: boolean;
  error: string | null;
  onRefresh: () => void;
}

export function ConfigView({ config, services, loading, error, onRefresh }: ConfigViewProps) {
  const [showRawJson, setShowRawJson] = useState(false);
  const [connectingPeer, setConnectingPeer] = useState<string | null>(null);
  const [connectMessage, setConnectMessage] = useState<{
    success: boolean;
    text: string;
  } | null>(null);

  // Dynamic Static Routes state
  const [routes, setRoutes] = useState<RouteItem[]>(config?.routes || []);
  const [routesLoading, setRoutesLoading] = useState(false);
  const [showAddRouteForm, setShowAddRouteForm] = useState(false);
  const [newPattern, setNewPattern] = useState('');
  const [newCla, setNewCla] = useState(config?.clas?.[0]?.name || '');
  const [newNextHop, setNewNextHop] = useState('');
  const [routeMessage, setRouteMessage] = useState<{
    success: boolean;
    text: string;
  } | null>(null);
  const [addingRoute, setAddingRoute] = useState(false);
  const [deletingPattern, setDeletingPattern] = useState<string | null>(null);

  // Peer Connection Statuses state
  const [peerStatuses, setPeerStatuses] = useState<Record<string, PeerStatusInfo>>({});

  // BPSec Security & Keystore state
  const [securityPolicy, setSecurityPolicy] = useState<VerifyPolicy>('warn');
  const [keys, setKeys] = useState<KeyInfo[]>([]);
  const [keysLoading, setKeysLoading] = useState(false);
  const [showAddKeyModal, setShowAddKeyModal] = useState(false);
  const [newKeyDirection, setNewKeyDirection] = useState<'outbound' | 'inbound'>('outbound');
  const [newKeyPattern, setNewKeyPattern] = useState('');
  const [newKeyLabel, setNewKeyLabel] = useState('');
  const [newKeySecret, setNewKeySecret] = useState('');
  const [showSecretText, setShowSecretText] = useState(false);
  const [keyMessage, setKeyMessage] = useState<{ success: boolean; text: string } | null>(null);
  const [submittingKey, setSubmittingKey] = useState(false);
  const [deletingKeyId, setDeletingKeyId] = useState<string | null>(null);

  const fetchSecurityPolicy = async () => {
    try {
      const res = await fetch('/api/v1/security/policy');
      if (res.ok) {
        const data = await res.json();
        if (data.policy) {
          setSecurityPolicy(data.policy);
        }
      }
    } catch {
      // Ignore
    }
  };

  const fetchKeys = async () => {
    try {
      setKeysLoading(true);
      const res = await fetch('/api/v1/security/keys');
      if (res.ok) {
        const data = await res.json();
        if (Array.isArray(data)) {
          setKeys(data);
        }
      }
    } catch {
      // Ignore
    } finally {
      setKeysLoading(false);
    }
  };

  const handleUpdatePolicy = async (newPolicy: VerifyPolicy) => {
    try {
      const res = await fetch('/api/v1/security/policy', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ policy: newPolicy }),
      });
      if (res.ok) {
        setSecurityPolicy(newPolicy);
        setKeyMessage({
          success: true,
          text: `Politique de vérification mise à jour en mode "${newPolicy.toUpperCase()}".`,
        });
      } else {
        const err = await res.json().catch(() => ({}));
        setKeyMessage({
          success: false,
          text: err.error || 'Échec de mise à jour de la politique.',
        });
      }
    } catch (e: any) {
      setKeyMessage({ success: false, text: e.message || 'Erreur réseau.' });
    }
  };

  const handleAddKey = async (e: Event) => {
    e.preventDefault();
    if (!newKeyPattern.trim() || !newKeySecret.trim()) {
      setKeyMessage({
        success: false,
        text: 'Le motif EID et la clé secrète sont obligatoires.',
      });
      return;
    }

    try {
      setSubmittingKey(true);
      setKeyMessage(null);
      const res = await fetch('/api/v1/security/keys', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          direction: newKeyDirection,
          pattern: newKeyPattern.trim(),
          label: newKeyLabel.trim() || null,
          secret: newKeySecret.trim(),
        }),
      });

      if (res.ok) {
        const created: KeyInfo = await res.json();
        setKeys((prev) => [...prev, created]);
        setNewKeyPattern('');
        setNewKeyLabel('');
        setNewKeySecret('');
        setShowAddKeyModal(false);
        setKeyMessage({
          success: true,
          text: `Clé ${created.direction.toUpperCase()} pour "${created.pattern}" enregistrée avec succès.`,
        });
      } else {
        const err = await res.json().catch(() => ({}));
        setKeyMessage({
          success: false,
          text: err.error || "Échec lors de l'enregistrement de la clé.",
        });
      }
    } catch (e: any) {
      setKeyMessage({ success: false, text: e.message || 'Erreur réseau.' });
    } finally {
      setSubmittingKey(false);
    }
  };

  const handleDeleteKey = async (id: string, pattern: string) => {
    if (!confirm(`Supprimer la clé de sécurité pour "${pattern}" ?`)) return;

    try {
      setDeletingKeyId(id);
      setKeyMessage(null);
      const res = await fetch(`/api/v1/security/keys/${encodeURIComponent(id)}`, {
        method: 'DELETE',
      });
      if (res.ok) {
        setKeys((prev) => prev.filter((k) => k.id !== id));
        setKeyMessage({
          success: true,
          text: `Clé pour "${pattern}" supprimée.`,
        });
      } else {
        const err = await res.json().catch(() => ({}));
        setKeyMessage({
          success: false,
          text: err.error || 'Échec lors de la suppression de la clé.',
        });
      }
    } catch (e: any) {
      setKeyMessage({ success: false, text: e.message || 'Erreur réseau.' });
    } finally {
      setDeletingKeyId(null);
    }
  };

  const generateRandomSecret = () => {
    const bytes = new Uint8Array(32);
    window.crypto.getRandomValues(bytes);
    const hex = Array.from(bytes)
      .map((b) => b.toString(16).padStart(2, '0'))
      .join('');
    setNewKeySecret(hex);
  };

  const fetchRoutes = async () => {
    try {
      setRoutesLoading(true);
      const res = await fetch('/api/v1/routes');
      if (res.ok) {
        const data = await res.json();
        if (Array.isArray(data)) {
          setRoutes(data);
        }
      }
    } catch {
      // Keep existing routes on failure
    } finally {
      setRoutesLoading(false);
    }
  };

  const fetchPeerStatuses = async () => {
    try {
      const res = await fetch('/api/v1/clas/status');
      if (res.ok) {
        const data: PeerStatusInfo[] = await res.json();
        const map: Record<string, PeerStatusInfo> = {};
        for (const item of data) {
          map[item.address] = item;
        }
        setPeerStatuses(map);
      }
    } catch {
      // Ignore network errors on polling
    }
  };

  useEffect(() => {
    fetchRoutes();
    fetchPeerStatuses();
    fetchSecurityPolicy();
    fetchKeys();

    const interval = setInterval(fetchPeerStatuses, 5000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    if (config?.routes && routes.length === 0) {
      setRoutes(config.routes);
    }
    if (!newCla && config?.clas?.[0]?.name) {
      setNewCla(config.clas[0].name);
    }
  }, [config]);

  const handleForceConnect = async (peerStr: string) => {
    setConnectingPeer(peerStr);
    setConnectMessage(null);
    try {
      const res = await fetch('/api/v1/clas/connect', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ peer: peerStr }),
      });
      const data = await res.json();
      if (res.ok) {
        setConnectMessage({
          success: true,
          text: `Successfully connected to peer ${peerStr}`,
        });
        setTimeout(() => setConnectMessage(null), 5000);
      } else {
        setConnectMessage({
          success: false,
          text: data.error || `Connection to ${peerStr} failed (HTTP ${res.status})`,
        });
      }
    } catch (err: any) {
      setConnectMessage({
        success: false,
        text: err.message || `Failed to connect to ${peerStr}`,
      });
    } finally {
      setConnectingPeer(null);
      fetchPeerStatuses();
    }
  };

  const handleAddRoute = async (e: Event) => {
    e.preventDefault();
    if (!newPattern.trim()) {
      setRouteMessage({ success: false, text: 'Route pattern cannot be empty' });
      return;
    }
    const targetCla = newCla.trim() || config?.clas?.[0]?.name || 'default';

    setAddingRoute(true);
    setRouteMessage(null);
    try {
      const body: any = {
        pattern: newPattern.trim(),
        cla: targetCla,
      };
      if (newNextHop.trim()) {
        body['next-hop'] = newNextHop.trim();
      }

      const res = await fetch('/api/v1/routes', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      const data = await res.json();
      if (res.ok) {
        setRouteMessage({
          success: true,
          text: `Route '${newPattern.trim()}' successfully added and injected into BPA RIB`,
        });
        setNewPattern('');
        setNewNextHop('');
        setShowAddRouteForm(false);
        await fetchRoutes();
        setTimeout(() => setRouteMessage(null), 5000);
      } else {
        setRouteMessage({
          success: false,
          text: data.error || `Failed to add route: HTTP ${res.status}`,
        });
      }
    } catch (err: any) {
      setRouteMessage({
        success: false,
        text: err.message || 'Failed to add static route',
      });
    } finally {
      setAddingRoute(false);
    }
  };

  const handleDeleteRoute = async (pattern: string) => {
    if (!confirm(`Are you sure you want to delete static route '${pattern}'?`)) {
      return;
    }

    setDeletingPattern(pattern);
    setRouteMessage(null);
    try {
      const res = await fetch(`/api/v1/routes?pattern=${encodeURIComponent(pattern)}`, {
        method: 'DELETE',
      });
      const data = await res.json();
      if (res.ok) {
        setRouteMessage({
          success: true,
          text: `Route '${pattern}' removed from BPA RIB and config`,
        });
        await fetchRoutes();
        setTimeout(() => setRouteMessage(null), 5000);
      } else {
        setRouteMessage({
          success: false,
          text: data.error || `Failed to delete route: HTTP ${res.status}`,
        });
      }
    } catch (err: any) {
      setRouteMessage({
        success: false,
        text: err.message || `Failed to delete route '${pattern}'`,
      });
    } finally {
      setDeletingPattern(null);
    }
  };

  if (loading && !config) {
    return (
      <div class="flex items-center justify-center p-16">
        <div class="animate-spin rounded-full h-12 w-12 border-b-2 border-emerald-600"></div>
      </div>
    );
  }

  if (error) {
    return (
      <div class="p-6 max-w-4xl mx-auto">
        <div class="bg-red-50 dark:bg-red-950/50 border border-red-200 dark:border-red-800 rounded-xl p-4 text-red-800 dark:text-red-200">
          <div class="flex items-center space-x-2 font-semibold">
            <span>⚠️</span>
            <span>Error loading configuration</span>
          </div>
          <p class="mt-2 text-sm text-red-700 dark:text-red-300">{error}</p>
          <button
            onClick={() => {
              onRefresh();
              fetchRoutes();
              fetchPeerStatuses();
            }}
            class="mt-4 px-4 py-2 bg-red-600 text-white rounded-lg text-sm font-medium hover:bg-red-700 transition cursor-pointer"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  if (!config) return null;

  return (
    <div class="space-y-6 max-w-6xl mx-auto py-6 px-4 sm:px-6">
      {/* Header bar */}
      <div class="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 bg-white dark:bg-slate-800 p-5 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700">
        <div>
          <h2 class="text-xl font-bold text-slate-900 dark:text-white flex items-center gap-2">
            <span>Station Overview</span>
            <span class="text-xs px-2.5 py-1 font-semibold rounded-md bg-emerald-100 text-emerald-800 dark:bg-emerald-900/50 dark:text-emerald-300">
              v{config.version}
            </span>
          </h2>
          <p class="text-sm text-slate-500 dark:text-slate-400 mt-1">
            Active BPv7 parameters, dynamic routing and convergence layer adapters
          </p>
        </div>
        <div class="flex items-center space-x-2">
          <button
            onClick={() => setShowRawJson(!showRawJson)}
            class="px-3 py-1.5 text-xs font-semibold rounded-lg border border-slate-300 dark:border-slate-600 hover:bg-slate-50 dark:hover:bg-slate-700 transition cursor-pointer"
          >
            {showRawJson ? 'Hide JSON' : 'View JSON'}
          </button>
          <button
            onClick={() => {
              onRefresh();
              fetchRoutes();
              fetchPeerStatuses();
            }}
            disabled={loading}
            class="px-3 py-1.5 text-xs font-semibold rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white transition flex items-center gap-1.5 cursor-pointer disabled:opacity-50"
          >
            <span>🔄</span>
            <span>Refresh</span>
          </button>
        </div>
      </div>

      {/* Raw JSON collapse */}
      {showRawJson && (
        <div class="bg-slate-900 text-slate-100 p-4 rounded-2xl overflow-x-auto text-xs font-mono border border-slate-700 shadow-inner">
          <pre>{JSON.stringify(config, null, 2)}</pre>
        </div>
      )}

      {/* Top 3 Cards Grid */}
      <div class="grid grid-cols-1 md:grid-cols-3 gap-6">
        {/* Card 1: Identity */}
        <div class="bg-white dark:bg-slate-800 p-5 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700 flex flex-col justify-between">
          <div>
            <div class="flex items-center justify-between">
              <span class="text-xs font-semibold uppercase tracking-wider text-slate-400">
                Radio Identity
              </span>
              <span class="text-base">📻</span>
            </div>
            <div class="mt-3">
              <div class="text-2xl font-black text-slate-900 dark:text-white">
                {config.callsign || 'NOCALL'}
              </div>
              <div class="text-xs font-mono text-emerald-600 dark:text-emerald-400 mt-1 break-all">
                {config.node_id}
              </div>
            </div>
          </div>
          {config.admin_endpoints.length > 0 && (
            <div class="mt-4 pt-3 border-t border-slate-100 dark:border-slate-700/60">
              <div class="text-xs text-slate-400 font-medium">Admin Endpoints:</div>
              <div class="flex flex-wrap gap-1.5 mt-1">
                {config.admin_endpoints.map((ep) => (
                  <span class="px-2 py-0.5 text-xs bg-slate-100 dark:bg-slate-700 rounded-md font-mono text-slate-700 dark:text-slate-300">
                    {ep}
                  </span>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Card 2: Storage */}
        <div class="bg-white dark:bg-slate-800 p-5 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700 flex flex-col justify-between">
          <div>
            <div class="flex items-center justify-between">
              <span class="text-xs font-semibold uppercase tracking-wider text-slate-400">
                Storage Engine
              </span>
              <span class="text-base">💾</span>
            </div>
            <div class="mt-3">
              <div class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-semibold bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800">
                <span class="w-2 h-2 rounded-full bg-emerald-500"></span>
                {config.storage.in_memory ? 'In-Memory Store (Volatile)' : 'Persistent SQLite'}
              </div>
              <div class="text-xs text-slate-500 dark:text-slate-400 mt-2 font-mono break-all">
                {config.storage.sqlite_path || 'Memory backend (no disk database)'}
              </div>
            </div>
          </div>
        </div>

        {/* Card 3: Network Interfaces */}
        <div class="bg-white dark:bg-slate-800 p-5 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700 flex flex-col justify-between">
          <div>
            <div class="flex items-center justify-between">
              <span class="text-xs font-semibold uppercase tracking-wider text-slate-400">
                Management Ports
              </span>
              <span class="text-base">🔌</span>
            </div>
            <div class="mt-3 space-y-2">
              <div class="flex items-center justify-between text-xs">
                <span class="text-slate-500 dark:text-slate-400">gRPC API:</span>
                <span class="font-mono font-medium text-slate-800 dark:text-slate-200">
                  {config.grpc.enabled ? config.grpc.address : 'Disabled'}
                </span>
              </div>
              <div class="flex items-center justify-between text-xs">
                <span class="text-slate-500 dark:text-slate-400">Web Dashboard:</span>
                <span class="font-mono font-medium text-slate-800 dark:text-slate-200">
                  {config.web?.address || 'Disabled'}
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Active DTN Services Card */}
      <div class="bg-white dark:bg-slate-800 p-6 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700">
        <div class="flex items-center justify-between border-b border-slate-100 dark:border-slate-700 pb-4 mb-4">
          <div class="flex items-center space-x-2">
            <span class="text-lg">🛎️</span>
            <h3 class="text-base font-bold text-slate-900 dark:text-white">
              Active DTN Services
            </h3>
          </div>
          <span class="px-2.5 py-0.5 text-xs font-semibold rounded-full bg-emerald-100 dark:bg-emerald-900/60 text-emerald-800 dark:text-emerald-300">
            {services?.length || 0} active
          </span>
        </div>

        {(!services || services.length === 0) ? (
          <p class="text-sm text-slate-500 dark:text-slate-400 italic">No services currently registered.</p>
        ) : (
          <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
            {services.map((svc) => (
              <div class="border border-slate-200 dark:border-slate-700 rounded-xl p-4 bg-slate-50/50 dark:bg-slate-800/50 hover:border-emerald-500/50 transition flex flex-col justify-between">
                <div class="flex items-start justify-between gap-2">
                  <span class="font-bold text-sm text-emerald-600 dark:text-emerald-400 font-mono break-all">
                    {svc.eid}
                  </span>
                  <span class={`px-2 py-0.5 text-[11px] rounded-md font-semibold whitespace-nowrap ${
                    svc.type.includes('Echo')
                      ? 'bg-purple-100 dark:bg-purple-900/60 text-purple-800 dark:text-purple-300'
                      : 'bg-sky-100 dark:bg-sky-900/60 text-sky-800 dark:text-sky-300'
                  }`}>
                    {svc.type}
                  </span>
                </div>
                <div class="mt-3 text-xs text-slate-500 dark:text-slate-400 flex items-center justify-between">
                  <span>Registered:</span>
                  <span class="font-mono text-[11px] text-slate-700 dark:text-slate-300">{svc.registered_at}</span>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Convergence Layer Adapters Card */}
      <div class="bg-white dark:bg-slate-800 p-6 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700">
        <div class="flex items-center justify-between border-b border-slate-100 dark:border-slate-700 pb-4 mb-4">
          <div class="flex items-center space-x-2">
            <span class="text-lg">📡</span>
            <h3 class="text-base font-bold text-slate-900 dark:text-white">
              Convergence Layer Adapters (CLAs)
            </h3>
          </div>
          <span class="px-2.5 py-0.5 text-xs font-semibold rounded-full bg-slate-100 dark:bg-slate-700 text-slate-600 dark:text-slate-300">
            {config.clas.length} configured
          </span>
        </div>

        {config.clas.length === 0 ? (
          <p class="text-sm text-slate-500 dark:text-slate-400 italic">No CLAs configured.</p>
        ) : (
          <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
            {config.clas.map((cla) => (
              <div class="border border-slate-200 dark:border-slate-700 rounded-xl p-4 bg-slate-50/50 dark:bg-slate-800/50 hover:border-emerald-500/50 transition">
                <div class="flex items-center justify-between">
                  <span class="font-bold text-sm text-slate-900 dark:text-white font-mono">
                    {cla.name}
                  </span>
                  <span class="px-2 py-0.5 text-xs rounded-md bg-emerald-100 dark:bg-emerald-900/60 text-emerald-800 dark:text-emerald-300 font-semibold uppercase">
                    {cla.type}
                  </span>
                </div>

                <div class="mt-3 text-xs space-y-1 font-mono text-slate-600 dark:text-slate-300">
                  {cla['control-addr'] && (
                    <div>Control Port: <span class="text-emerald-600 dark:text-emerald-400">{cla['control-addr']}</span></div>
                  )}
                  {cla['data-addr'] && (
                    <div>Data Port: <span class="text-emerald-600 dark:text-emerald-400">{cla['data-addr']}</span></div>
                  )}
                  {cla.bandwidth && (
                    <div>Bandwidth: {cla.bandwidth} Hz</div>
                  )}
                  {cla.address && (
                    <div>Bind Address: {cla.address}</div>
                  )}
                  {cla['engine-id'] && (
                    <div>LTP Engine ID: {cla['engine-id']}</div>
                  )}
                </div>

                {/* Peers list */}
                {cla.peers && cla.peers.length > 0 && (
                  <div class="mt-3 pt-3 border-t border-slate-200 dark:border-slate-700 text-xs">
                    <span class="text-slate-400 font-medium">Configured Peers:</span>

                    {connectMessage && (
                      <div
                        class={`mt-2 p-2.5 rounded-lg text-xs font-medium border flex items-center justify-between ${
                          connectMessage.success
                            ? 'bg-emerald-50 dark:bg-emerald-950/60 border-emerald-300 dark:border-emerald-800 text-emerald-800 dark:text-emerald-300'
                            : 'bg-red-50 dark:bg-red-950/60 border-red-300 dark:border-red-800 text-red-800 dark:text-red-300'
                        }`}
                      >
                        <span>{connectMessage.success ? '✓' : '⚠️'} {connectMessage.text}</span>
                        <button
                          onClick={() => setConnectMessage(null)}
                          class="text-xs hover:underline ml-2 cursor-pointer"
                        >
                          ✕
                        </button>
                      </div>
                    )}

                    <div class="mt-1.5 space-y-2">
                      {cla.peers.map((peer: any) => {
                        const peerTarget =
                          typeof peer === 'string'
                            ? peer
                            : peer.address || peer.callsign || peer['node-id'] || '';
                        const isConnecting = connectingPeer === peerTarget;
                        const statusObj = peerStatuses[peerTarget] || (typeof peer === 'string' ? peerStatuses[peer] : undefined);
                        const status = statusObj?.status || 'disconnected';

                        return (
                          <div class="bg-white dark:bg-slate-900/80 p-2.5 rounded-lg border border-slate-200 dark:border-slate-700/80">
                            <div class="flex items-center justify-between font-mono font-semibold">
                              <div class="flex items-center space-x-2">
                                <span class="text-slate-800 dark:text-slate-200">
                                  {peer.callsign || peer}
                                </span>
                                {peer['node-id'] && (
                                  <span class="text-emerald-600 dark:text-emerald-400 text-[11px]">
                                    {peer['node-id']}
                                  </span>
                                )}
                              </div>

                              <div class="flex items-center space-x-2">
                                {/* Peer Connection Status Badge */}
                                <span
                                  class={`px-2 py-0.5 text-[11px] font-semibold rounded-md flex items-center gap-1 border ${
                                    status === 'connected'
                                      ? 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border-emerald-300 dark:border-emerald-800'
                                      : status === 'connecting'
                                      ? 'bg-amber-50 dark:bg-amber-950/60 text-amber-700 dark:text-amber-300 border-amber-300 dark:border-amber-800'
                                      : 'bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-400 border-slate-300 dark:border-slate-700'
                                  }`}
                                >
                                  <span>{status === 'connected' ? '🟢' : status === 'connecting' ? '🟡' : '🔴'}</span>
                                  <span class="capitalize">{status}</span>
                                </span>

                                {/* Manual Connect Button */}
                                <button
                                  onClick={() => handleForceConnect(peerTarget)}
                                  disabled={isConnecting}
                                  title={`Force connection to peer ${peerTarget}`}
                                  class="px-2.5 py-1 text-xs font-semibold rounded-lg bg-emerald-50 dark:bg-emerald-950/60 hover:bg-emerald-100 dark:hover:bg-emerald-900/80 text-emerald-700 dark:text-emerald-300 border border-emerald-300 dark:border-emerald-700 transition flex items-center gap-1.5 cursor-pointer disabled:opacity-50"
                                >
                                  <span>{isConnecting ? '⏳' : '✋'}</span>
                                  <span>{isConnecting ? 'Connecting...' : 'Connect'}</span>
                                </button>
                              </div>
                            </div>
                            {peer.schedule && (
                              <div class="mt-1 text-[11px] text-slate-500 dark:text-slate-400 flex items-center gap-2">
                                <span>⏱️ Schedule:</span>
                                <span class="font-mono bg-slate-100 dark:bg-slate-800 px-1.5 py-0.5 rounded text-emerald-600 dark:text-emerald-300">
                                  {peer.schedule.cron || peer.schedule.type} ({peer.schedule.duration || 'no duration'})
                                </span>
                              </div>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Dynamic Static Routes Card */}
      <div class="bg-white dark:bg-slate-800 p-6 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700">
        <div class="flex items-center justify-between border-b border-slate-100 dark:border-slate-700 pb-4 mb-4">
          <div class="flex items-center space-x-2">
            <span class="text-lg">🗺️</span>
            <h3 class="text-base font-bold text-slate-900 dark:text-white">Static Routes</h3>
          </div>
          <div class="flex items-center space-x-2">
            <span class="px-2.5 py-0.5 text-xs font-semibold rounded-full bg-slate-100 dark:bg-slate-700 text-slate-600 dark:text-slate-300">
              {routes.length} active
            </span>
            <button
              onClick={() => {
                setShowAddRouteForm(!showAddRouteForm);
                setRouteMessage(null);
              }}
              class="px-3 py-1 text-xs font-semibold rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white transition flex items-center gap-1 cursor-pointer"
            >
              <span>{showAddRouteForm ? '✕ Cancel' : '+ Add Route'}</span>
            </button>
          </div>
        </div>

        {routeMessage && (
          <div
            class={`mb-4 p-3 rounded-xl text-xs font-medium border flex items-center justify-between ${
              routeMessage.success
                ? 'bg-emerald-50 dark:bg-emerald-950/60 border-emerald-300 dark:border-emerald-800 text-emerald-800 dark:text-emerald-300'
                : 'bg-red-50 dark:bg-red-950/60 border-red-300 dark:border-red-800 text-red-800 dark:text-red-300'
            }`}
          >
            <span>{routeMessage.success ? '✓' : '⚠️'} {routeMessage.text}</span>
            <button
              onClick={() => setRouteMessage(null)}
              class="text-xs hover:underline ml-2 cursor-pointer"
            >
              ✕
            </button>
          </div>
        )}

        {/* Add Route Inline Form */}
        {showAddRouteForm && (
          <form
            onSubmit={handleAddRoute}
            class="mb-6 p-4 rounded-xl border border-emerald-300 dark:border-emerald-800 bg-emerald-50/40 dark:bg-emerald-950/30 space-y-3"
          >
            <div class="font-semibold text-xs text-emerald-800 dark:text-emerald-300">
              Add New Static Route
            </div>
            <div class="grid grid-cols-1 sm:grid-cols-3 gap-3">
              <div>
                <label for="route-pattern-input" class="block text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1">
                  Destination Pattern *
                </label>
                <input
                  id="route-pattern-input"
                  type="text"
                  required
                  placeholder="e.g. dtn://f4jxq-9/* or dtn:**"
                  value={newPattern}
                  onInput={(e: any) => setNewPattern(e.target.value)}
                  class="w-full text-xs font-mono px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-900 text-slate-800 dark:text-slate-200 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                />
              </div>

              <div>
                <label for="route-cla-select" class="block text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1">
                  Target CLA *
                </label>
                {config.clas.length > 0 ? (
                  <select
                    id="route-cla-select"
                    value={newCla}
                    onChange={(e: any) => setNewCla(e.target.value)}
                    class="w-full text-xs font-mono px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-900 text-slate-800 dark:text-slate-200 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                  >
                    {config.clas.map((c) => (
                      <option value={c.name}>{c.name} ({c.type})</option>
                    ))}
                  </select>
                ) : (
                  <input
                    id="route-cla-select"
                    type="text"
                    required
                    placeholder="e.g. tcpcl-f4jxq"
                    value={newCla}
                    onInput={(e: any) => setNewCla(e.target.value)}
                    class="w-full text-xs font-mono px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-900 text-slate-800 dark:text-slate-200 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                  />
                )}
              </div>

              <div>
                <label for="route-nexthop-input" class="block text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1">
                  Next-Hop EID (optional)
                </label>
                <input
                  id="route-nexthop-input"
                  type="text"
                  placeholder="e.g. dtn://f4jxq/"
                  value={newNextHop}
                  onInput={(e: any) => setNewNextHop(e.target.value)}
                  class="w-full text-xs font-mono px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-900 text-slate-800 dark:text-slate-200 focus:outline-none focus:ring-1 focus:ring-emerald-500"
                />
              </div>
            </div>

            <div class="flex items-center justify-end space-x-2 pt-2">
              <button
                type="button"
                onClick={() => setShowAddRouteForm(false)}
                class="px-3 py-1.5 text-xs rounded-lg border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-800 transition cursor-pointer"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={addingRoute}
                class="px-4 py-1.5 text-xs font-semibold rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white transition cursor-pointer disabled:opacity-50"
              >
                {addingRoute ? 'Injecting into RIB...' : 'Add Route'}
              </button>
            </div>
          </form>
        )}

        {routes.length === 0 ? (
          <p class="text-sm text-slate-500 dark:text-slate-400 italic">No static routes configured.</p>
        ) : (
          <div class="overflow-x-auto">
            <table class="w-full text-left text-xs font-mono divide-y divide-slate-100 dark:divide-slate-700">
              <thead>
                <tr class="text-slate-400 font-semibold border-b border-slate-200 dark:border-slate-700">
                  <th class="py-2 px-3">Pattern</th>
                  <th class="py-2 px-3">Next-Hop</th>
                  <th class="py-2 px-3">CLA</th>
                  <th class="py-2 px-3 text-right">Actions</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
                {routes.map((route) => {
                  const isDeleting = deletingPattern === route.pattern;
                  const nextHop = route['next-hop'] || route.next_hop || 'direct';

                  return (
                    <tr class="hover:bg-slate-50 dark:hover:bg-slate-800/60 transition">
                      <td class="py-2.5 px-3 font-semibold text-slate-800 dark:text-slate-200">
                        {route.pattern}
                      </td>
                      <td class="py-2.5 px-3 text-emerald-600 dark:text-emerald-400">
                        {nextHop}
                      </td>
                      <td class="py-2.5 px-3 text-slate-600 dark:text-slate-400">
                        via {route.cla}
                      </td>
                      <td class="py-2.5 px-3 text-right">
                        <button
                          onClick={() => handleDeleteRoute(route.pattern)}
                          disabled={isDeleting}
                          title={`Delete static route ${route.pattern}`}
                          class="px-2 py-1 text-[11px] font-semibold rounded bg-red-50 dark:bg-red-950/60 hover:bg-red-100 dark:hover:bg-red-900/80 text-red-700 dark:text-red-300 border border-red-300 dark:border-red-800 transition cursor-pointer disabled:opacity-50"
                        >
                          {isDeleting ? 'Removing...' : '🗑️ Delete'}
                        </button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* =========================================================================
          SECTION: BPSEC AUTHENTICATION & KEYSTORE (RFC 9103 / RFC 9172)
          ========================================================================= */}
      <div class="bg-white dark:bg-slate-800 rounded-2xl p-6 shadow-xs border border-slate-200 dark:border-slate-700">
        <div class="flex flex-col sm:flex-row sm:items-center justify-between pb-4 border-b border-slate-100 dark:border-slate-700 gap-3">
          <div class="flex items-center space-x-2">
            <span class="text-xl">🛡️</span>
            <div>
              <h3 class="text-base font-bold text-slate-900 dark:text-white">
                Sécurité & Authentification BPSec (RFC 9103 / RFC 9172)
              </h3>
              <p class="text-xs text-slate-500 dark:text-slate-400">
                Gestion des clés HMAC-SHA256 pour signature automatique et vérification d'intégrité (sans chiffrement).
              </p>
            </div>
          </div>
          <div class="flex items-center space-x-2">
            <span class="px-2.5 py-0.5 text-xs font-semibold rounded-full bg-slate-100 dark:bg-slate-700 text-slate-600 dark:text-slate-300">
              {keys.length} clé{keys.length > 1 ? 's' : ''}
            </span>
            <button
              onClick={() => {
                setShowAddKeyModal(!showAddKeyModal);
                setKeyMessage(null);
              }}
              class="px-3 py-1 text-xs font-semibold rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white transition flex items-center gap-1 cursor-pointer"
            >
              <span>{showAddKeyModal ? '✕ Fermer' : '+ Ajouter une clé'}</span>
            </button>
          </div>
        </div>

        {keyMessage && (
          <div
            class={`my-4 p-3 rounded-xl text-xs font-medium border flex items-center justify-between ${
              keyMessage.success
                ? 'bg-emerald-50 dark:bg-emerald-950/60 border-emerald-300 dark:border-emerald-800 text-emerald-800 dark:text-emerald-300'
                : 'bg-red-50 dark:bg-red-950/60 border-red-300 dark:border-red-800 text-red-800 dark:text-red-300'
            }`}
          >
            <span>{keyMessage.success ? '✓' : '⚠️'} {keyMessage.text}</span>
            <button
              onClick={() => setKeyMessage(null)}
              class="text-xs hover:underline ml-2 cursor-pointer"
            >
              ✕
            </button>
          </div>
        )}

        {/* Policy Selector & Info Callout */}
        <div class="mt-4 p-4 rounded-xl bg-slate-50 dark:bg-slate-900/50 border border-slate-200 dark:border-slate-700 grid grid-cols-1 md:grid-cols-3 gap-4 items-center">
          <div class="md:col-span-2">
            <label class="block text-xs font-bold text-slate-800 dark:text-slate-200 mb-1">
              Politique de vérification à l'ingestion
            </label>
            <p class="text-[11px] text-slate-500 dark:text-slate-400">
              Contrôle l'acceptation ou le rejet des paquets entrants lors d'une altération ou signature invalide.
            </p>
          </div>
          <div>
            <select
              value={securityPolicy}
              onChange={(e: any) => handleUpdatePolicy(e.target.value)}
              class="w-full text-xs font-semibold px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-800 text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-emerald-500"
            >
              <option value="warn">Avertissement (Warn) — Recommandé</option>
              <option value="strict">Strict (Rejeter signatures invalides)</option>
              <option value="ignore">Ignorer la vérification (Ignore)</option>
            </select>
          </div>
        </div>

        {/* Add Key Form Modal / Drawer */}
        {showAddKeyModal && (
          <form
            onSubmit={handleAddKey}
            class="my-4 p-4 rounded-xl border border-emerald-300 dark:border-emerald-800 bg-emerald-50/40 dark:bg-emerald-950/30 space-y-4"
          >
            <div class="flex items-center justify-between">
              <span class="font-semibold text-xs text-emerald-800 dark:text-emerald-300">
                Configuration d'une nouvelle clé BPSec BIB
              </span>
              <span class="text-[10px] text-slate-500 dark:text-slate-400">
                Fichier local protégé (POSIX 0600) — Secrets jamais renvoyés par l'API
              </span>
            </div>

            <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
              {/* Direction */}
              <div>
                <label class="block text-[11px] font-medium text-slate-700 dark:text-slate-300 mb-1">
                  Direction de la clé *
                </label>
                <div class="flex items-center space-x-2">
                  <button
                    type="button"
                    onClick={() => setNewKeyDirection('outbound')}
                    class={`flex-1 py-1.5 px-3 rounded-lg text-xs font-medium border cursor-pointer transition ${
                      newKeyDirection === 'outbound'
                        ? 'bg-emerald-600 text-white border-emerald-600 shadow-xs'
                        : 'bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-300 border-slate-300 dark:border-slate-600 hover:bg-slate-50'
                    }`}
                  >
                    📤 Sortante (Destination)
                  </button>
                  <button
                    type="button"
                    onClick={() => setNewKeyDirection('inbound')}
                    class={`flex-1 py-1.5 px-3 rounded-lg text-xs font-medium border cursor-pointer transition ${
                      newKeyDirection === 'inbound'
                        ? 'bg-blue-600 text-white border-blue-600 shadow-xs'
                        : 'bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-300 border-slate-300 dark:border-slate-600 hover:bg-slate-50'
                    }`}
                  >
                    📥 Entrante (Émetteur)
                  </button>
                </div>
                <p class="text-[10px] text-slate-500 dark:text-slate-400 mt-1">
                  {newKeyDirection === 'outbound'
                    ? 'Utilisée pour signer les bundles envoyés vers les nœuds correspondant au motif.'
                    : 'Utilisée pour vérifier les bundles reçus depuis les émetteurs correspondant au motif.'}
                </p>
              </div>

              {/* Label */}
              <div>
                <label class="block text-[11px] font-medium text-slate-700 dark:text-slate-300 mb-1">
                  Libellé / Station (optionnel)
                </label>
                <input
                  type="text"
                  placeholder="ex: Station F5KUV ou Club Relais"
                  value={newKeyLabel}
                  onInput={(e: any) => setNewKeyLabel(e.target.value)}
                  class="w-full text-xs px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-800 text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-emerald-500"
                />
              </div>
            </div>

            {/* Pattern input and presets */}
            <div>
              <div class="flex items-center justify-between mb-1">
                <label class="text-[11px] font-medium text-slate-700 dark:text-slate-300">
                  Motif d'EID (Pattern) *
                </label>
                <div class="flex items-center space-x-1">
                  <span class="text-[10px] text-slate-400">Exemples :</span>
                  {['dtn://*', 'dtn://f5kuv/*', 'dtn://*/chat', 'ipn:0.1.*'].map((pat) => (
                    <button
                      type="button"
                      onClick={() => setNewKeyPattern(pat)}
                      class="text-[10px] font-mono px-1.5 py-0.5 rounded bg-slate-200 dark:bg-slate-700 hover:bg-emerald-100 dark:hover:bg-emerald-900 text-slate-700 dark:text-slate-300 cursor-pointer"
                    >
                      {pat}
                    </button>
                  ))}
                </div>
              </div>
              <input
                type="text"
                placeholder="ex: dtn://f5kuv/* ou dtn://*/chat ou dtn://*"
                value={newKeyPattern}
                onInput={(e: any) => setNewKeyPattern(e.target.value)}
                required
                class="w-full font-mono text-xs px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-800 text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-emerald-500"
              />
            </div>

            {/* Secret key input and generator */}
            <div>
              <div class="flex items-center justify-between mb-1">
                <label class="text-[11px] font-medium text-slate-700 dark:text-slate-300">
                  Clé Secrète HMAC-SHA256 (Passphrase ou Hex 256 bits) *
                </label>
                <div class="flex items-center space-x-2">
                  <button
                    type="button"
                    onClick={generateRandomSecret}
                    class="text-[11px] text-emerald-600 dark:text-emerald-400 hover:underline cursor-pointer"
                  >
                    🎲 Générer aléatoire (256-bit Hex)
                  </button>
                  <button
                    type="button"
                    onClick={() => setShowSecretText(!showSecretText)}
                    class="text-[11px] text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 cursor-pointer"
                  >
                    {showSecretText ? 'Masquer' : 'Afficher'}
                  </button>
                </div>
              </div>
              <input
                type={showSecretText ? 'text' : 'password'}
                placeholder="Entrez une phrase secrète partagée ou une clé hexadécimale (64 caractères)"
                value={newKeySecret}
                onInput={(e: any) => setNewKeySecret(e.target.value)}
                required
                class="w-full font-mono text-xs px-3 py-1.5 rounded-lg border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-800 text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-emerald-500"
              />
            </div>

            <div class="flex items-center justify-end space-x-2 pt-2">
              <button
                type="button"
                onClick={() => setShowAddKeyModal(false)}
                class="px-3 py-1.5 text-xs font-semibold rounded-lg border border-slate-300 dark:border-slate-600 text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700 cursor-pointer"
              >
                Annuler
              </button>
              <button
                type="submit"
                disabled={submittingKey}
                class="px-4 py-1.5 text-xs font-semibold rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white transition cursor-pointer disabled:opacity-50"
              >
                {submittingKey ? 'Enregistrement...' : 'Enregistrer la clé'}
              </button>
            </div>
          </form>
        )}

        {/* Tables: Outbound & Inbound Keys */}
        <div class="mt-6 space-y-6">
          {/* Outbound Keys Table */}
          <div>
            <div class="flex items-center space-x-2 mb-2">
              <span class="text-sm">📤</span>
              <h4 class="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">
                Clés sortantes (Signature automatique selon le destinataire)
              </h4>
            </div>

            {keys.filter((k) => k.direction === 'outbound').length === 0 ? (
              <p class="text-xs text-slate-400 italic bg-slate-50 dark:bg-slate-900/30 p-3 rounded-lg">
                Aucune clé sortante configurée. Les paquets émis ne seront pas signés BPSec.
              </p>
            ) : (
              <div class="overflow-x-auto border border-slate-200 dark:border-slate-700 rounded-xl">
                <table class="w-full text-left text-xs font-mono divide-y divide-slate-100 dark:divide-slate-700">
                  <thead class="bg-slate-50 dark:bg-slate-900/50 text-slate-500">
                    <tr>
                      <th class="py-2 px-3">Motif Destination</th>
                      <th class="py-2 px-3">Libellé</th>
                      <th class="py-2 px-3">Empreinte SHA-256</th>
                      <th class="py-2 px-3">Créée le</th>
                      <th class="py-2 px-3 text-right">Action</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
                    {keys
                      .filter((k) => k.direction === 'outbound')
                      .map((k) => (
                        <tr class="hover:bg-slate-50 dark:hover:bg-slate-800/60 transition">
                          <td class="py-2.5 px-3 font-semibold text-emerald-600 dark:text-emerald-400">
                            {k.pattern}
                          </td>
                          <td class="py-2.5 px-3 text-slate-700 dark:text-slate-300">
                            {k.label || '—'}
                          </td>
                          <td class="py-2.5 px-3 text-slate-500 dark:text-slate-400 text-[11px]">
                            {k.fingerprint}
                          </td>
                          <td class="py-2.5 px-3 text-slate-400 text-[11px]">
                            {k.created_at ? k.created_at.substring(0, 10) : '—'}
                          </td>
                          <td class="py-2.5 px-3 text-right">
                            <button
                              onClick={() => handleDeleteKey(k.id, k.pattern)}
                              disabled={deletingKeyId === k.id}
                              class="px-2 py-1 text-[11px] font-semibold rounded bg-red-50 dark:bg-red-950/60 hover:bg-red-100 dark:hover:bg-red-900/80 text-red-700 dark:text-red-300 border border-red-300 dark:border-red-800 transition cursor-pointer disabled:opacity-50"
                            >
                              {deletingKeyId === k.id ? '...' : '🗑️ Supprimer'}
                            </button>
                          </td>
                        </tr>
                      ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>

          {/* Inbound Keys Table */}
          <div>
            <div class="flex items-center space-x-2 mb-2">
              <span class="text-sm">📥</span>
              <h4 class="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">
                Clés entrantes (Vérification cryptographique selon l'émetteur)
              </h4>
            </div>

            {keys.filter((k) => k.direction === 'inbound').length === 0 ? (
              <p class="text-xs text-slate-400 italic bg-slate-50 dark:bg-slate-900/30 p-3 rounded-lg">
                Aucune clé entrante configurée. Les paquets reçus porteront l'état "Non signé" ou "Clé manquante".
              </p>
            ) : (
              <div class="overflow-x-auto border border-slate-200 dark:border-slate-700 rounded-xl">
                <table class="w-full text-left text-xs font-mono divide-y divide-slate-100 dark:divide-slate-700">
                  <thead class="bg-slate-50 dark:bg-slate-900/50 text-slate-500">
                    <tr>
                      <th class="py-2 px-3">Motif Émetteur</th>
                      <th class="py-2 px-3">Libellé</th>
                      <th class="py-2 px-3">Empreinte SHA-256</th>
                      <th class="py-2 px-3">Créée le</th>
                      <th class="py-2 px-3 text-right">Action</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
                    {keys
                      .filter((k) => k.direction === 'inbound')
                      .map((k) => (
                        <tr class="hover:bg-slate-50 dark:hover:bg-slate-800/60 transition">
                          <td class="py-2.5 px-3 font-semibold text-blue-600 dark:text-blue-400">
                            {k.pattern}
                          </td>
                          <td class="py-2.5 px-3 text-slate-700 dark:text-slate-300">
                            {k.label || '—'}
                          </td>
                          <td class="py-2.5 px-3 text-slate-500 dark:text-slate-400 text-[11px]">
                            {k.fingerprint}
                          </td>
                          <td class="py-2.5 px-3 text-slate-400 text-[11px]">
                            {k.created_at ? k.created_at.substring(0, 10) : '—'}
                          </td>
                          <td class="py-2.5 px-3 text-right">
                            <button
                              onClick={() => handleDeleteKey(k.id, k.pattern)}
                              disabled={deletingKeyId === k.id}
                              class="px-2 py-1 text-[11px] font-semibold rounded bg-red-50 dark:bg-red-950/60 hover:bg-red-100 dark:hover:bg-red-900/80 text-red-700 dark:text-red-300 border border-red-300 dark:border-red-800 transition cursor-pointer disabled:opacity-50"
                            >
                              {deletingKeyId === k.id ? '...' : '🗑️ Supprimer'}
                            </button>
                          </td>
                        </tr>
                      ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
