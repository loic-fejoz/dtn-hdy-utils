import { h } from 'preact';
import { useState } from 'preact/hooks';

export interface ActiveService {
  eid: string;
  type: string;
  registered_at: string;
}

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
  routes: Array<{
    pattern: string;
    cla: string;
    [key: string]: any;
  }>;
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
            onClick={onRefresh}
            class="mt-4 px-4 py-2 bg-red-600 text-white rounded-lg text-sm font-medium hover:bg-red-700 transition"
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
            Active BPv7 parameters and convergence layer adapters in read-only mode
          </p>
        </div>
        <div class="flex items-center space-x-2">
          <button
            onClick={() => setShowRawJson(!showRawJson)}
            class="px-3 py-1.5 text-xs font-semibold rounded-lg border border-slate-300 dark:border-slate-600 hover:bg-slate-50 dark:hover:bg-slate-700 transition"
          >
            {showRawJson ? 'Hide JSON' : 'View JSON'}
          </button>
          <button
            onClick={onRefresh}
            disabled={loading}
            class="px-3 py-1.5 text-xs font-semibold rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white transition flex items-center gap-1.5"
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
                          class="text-xs hover:underline ml-2"
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

      {/* Static Routes Card */}
      {config.routes && config.routes.length > 0 && (
        <div class="bg-white dark:bg-slate-800 p-6 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700">
          <div class="flex items-center space-x-2 border-b border-slate-100 dark:border-slate-700 pb-4 mb-4">
            <span class="text-lg">🗺️</span>
            <h3 class="text-base font-bold text-slate-900 dark:text-white">Static Routes</h3>
          </div>
          <div class="divide-y divide-slate-100 dark:divide-slate-700 text-xs font-mono">
            {config.routes.map((route) => (
              <div class="py-2.5 flex items-center justify-between">
                <span class="text-slate-700 dark:text-slate-300">{route.pattern}</span>
                <span class="text-emerald-600 dark:text-emerald-400 font-semibold">via {route.cla}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
