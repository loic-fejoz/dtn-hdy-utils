import { h } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import { renderMarkdown } from '../utils/markdown';
import { SecurityBadge } from './SecurityBadge';

export interface BundleSummary {
  id: string;
  source: string;
  destination: string;
  creation_time: number;
  created_at: string;
  lifetime_secs: number;
  size: number;
  status: string;
  auth_status?: string;
}

export interface BundleDetail extends BundleSummary {
  mime_type: string;
  is_binary: boolean;
  payload_size: number;
  storage_name: string | null;
  payload_preview: string | null;
  hex_dump: string | null;
}

interface BundlesViewProps {
  onRefresh?: () => void;
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

function formatLifetime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.round(secs / 60)}m`;
  if (secs < 86400) return `${(secs / 3600).toFixed(1)}h`;
  return `${(secs / 86400).toFixed(1)}d`;
}

function formatShortId(id: string): string {
  if (id.length <= 14) return id;
  return `${id.slice(0, 8)}…${id.slice(-4)}`;
}

function getStatusBadge(status: string) {
  switch (status.toLowerCase()) {
    case 'new':
      return 'bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300 border-emerald-300 dark:border-emerald-800';
    case 'waiting':
      return 'bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300 border-amber-300 dark:border-amber-800';
    case 'forwardpending':
      return 'bg-blue-100 text-blue-800 dark:bg-blue-950 dark:text-blue-300 border-blue-300 dark:border-blue-800';
    case 'waitingforservice':
      return 'bg-purple-100 text-purple-800 dark:bg-purple-950 dark:text-purple-300 border-purple-300 dark:border-purple-800';
    case 'completed':
      return 'bg-teal-100 text-teal-800 dark:bg-teal-950 dark:text-teal-300 border-teal-300 dark:border-teal-800';
    default:
      return 'bg-slate-100 text-slate-800 dark:bg-slate-800 dark:text-slate-300 border-slate-300 dark:border-slate-700';
  }
}

function getMimeBadge(mime: string) {
  if (mime.startsWith('image/')) {
    return 'bg-pink-100 text-pink-800 dark:bg-pink-950 dark:text-pink-300 border-pink-300';
  }
  if (mime.startsWith('video/')) {
    return 'bg-purple-100 text-purple-800 dark:bg-purple-950 dark:text-purple-300 border-purple-300';
  }
  if (mime.startsWith('audio/')) {
    return 'bg-indigo-100 text-indigo-800 dark:bg-indigo-950 dark:text-indigo-300 border-indigo-300';
  }
  if (mime.includes('pdf')) {
    return 'bg-rose-100 text-rose-800 dark:bg-rose-950 dark:text-rose-300 border-rose-300';
  }
  if (
    mime.includes('zip') ||
    mime.includes('compressed') ||
    mime.includes('tar') ||
    mime.includes('gzip') ||
    mime.includes('zstd') ||
    mime.includes('xz') ||
    mime.includes('bzip')
  ) {
    return 'bg-cyan-100 text-cyan-800 dark:bg-cyan-950 dark:text-cyan-300 border-cyan-300';
  }
  if (mime.includes('sqlite') || mime.includes('sql')) {
    return 'bg-blue-100 text-blue-800 dark:bg-blue-950 dark:text-blue-300 border-blue-300';
  }
  if (mime.includes('gpx') || mime.includes('kml') || mime.includes('geo')) {
    return 'bg-teal-100 text-teal-800 dark:bg-teal-950 dark:text-teal-300 border-teal-300';
  }
  if (mime.includes('senml') || mime.includes('json')) {
    return 'bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300 border-amber-300';
  }
  if (mime.includes('markdown') || mime.startsWith('text/')) {
    return 'bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300 border-emerald-300';
  }
  return 'bg-slate-100 text-slate-800 dark:bg-slate-800 dark:text-slate-300 border-slate-300';
}

export function BundlesView({ onRefresh }: BundlesViewProps) {
  const [bundles, setBundles] = useState<BundleSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [searchTerm, setSearchTerm] = useState('');
  const [statusFilter, setStatusFilter] = useState('ALL');

  // Inspection modal state
  const [selectedBundleId, setSelectedBundleId] = useState<string | null>(null);
  const [bundleDetail, setBundleDetail] = useState<BundleDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [detailViewMode, setDetailViewMode] = useState<'preview' | 'hex' | 'raw'>('preview');

  // Deletion modal state
  const [bundleToDelete, setBundleToDelete] = useState<BundleSummary | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [actionSuccess, setActionSuccess] = useState<string | null>(null);

  const fetchBundles = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch('/api/v1/bundles');
      if (!res.ok) {
        throw new Error(`Failed to fetch bundles: HTTP ${res.status}`);
      }
      const data: BundleSummary[] = await res.json();
      setBundles(data);
    } catch (err: any) {
      setError(err.message || 'Error loading bundles');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchBundles();
  }, []);

  // Escape key closes modals
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (bundleToDelete) {
          setBundleToDelete(null);
        } else if (selectedBundleId) {
          setSelectedBundleId(null);
          setBundleDetail(null);
        }
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [bundleToDelete, selectedBundleId]);

  // Open inspection drawer / modal
  const handleInspect = async (id: string) => {
    setSelectedBundleId(id);
    setLoadingDetail(true);
    setDetailError(null);
    setDetailViewMode('preview');
    try {
      const res = await fetch(`/api/v1/bundles?id=${encodeURIComponent(id)}`);
      if (!res.ok) {
        throw new Error(`Failed to load bundle details: HTTP ${res.status}`);
      }
      const data: BundleDetail = await res.json();
      setBundleDetail(data);
    } catch (err: any) {
      setDetailError(err.message || 'Error loading bundle details');
    } finally {
      setLoadingDetail(false);
    }
  };

  // Perform deletion
  const handleConfirmDelete = async () => {
    if (!bundleToDelete) return;
    setDeleting(true);
    try {
      const res = await fetch(`/api/v1/bundles?id=${encodeURIComponent(bundleToDelete.id)}`, {
        method: 'DELETE',
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data.error || `HTTP ${res.status}`);
      }
      setActionSuccess(`Bundle ${bundleToDelete.id} deleted successfully`);
      setBundleToDelete(null);
      if (selectedBundleId === bundleToDelete.id) {
        setSelectedBundleId(null);
        setBundleDetail(null);
      }
      fetchBundles();
      if (onRefresh) onRefresh();
    } catch (err: any) {
      setError(err.message || 'Failed to delete bundle');
    } finally {
      setDeleting(false);
    }
  };

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text);
    setActionSuccess('Copied to clipboard!');
    setTimeout(() => setActionSuccess(null), 2500);
  };

  // Filtered bundles
  const filteredBundles = bundles.filter((b) => {
    const term = searchTerm.toLowerCase();
    const matchesSearch =
      b.id.toLowerCase().includes(term) ||
      b.source.toLowerCase().includes(term) ||
      b.destination.toLowerCase().includes(term);
    const matchesStatus =
      statusFilter === 'ALL' || b.status.toUpperCase() === statusFilter.toUpperCase();
    return matchesSearch && matchesStatus;
  });

  return (
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-6">
      {/* Top Header & Actions */}
      <div class="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 class="text-2xl font-bold tracking-tight text-slate-900 dark:text-white flex items-center gap-2">
            <span>📦</span>
            <span>Bundle Storage Inspector</span>
          </h1>
          <p class="text-sm text-slate-500 dark:text-slate-400 mt-1">
            Real-time unified management view of stored, queued, and dispatched bundles in BPA storage.
          </p>
        </div>

        <div class="flex items-center space-x-3">
          <button
            onClick={() => {
              fetchBundles();
              if (onRefresh) onRefresh();
            }}
            disabled={loading}
            class="px-4 py-2 text-sm font-medium rounded-lg border border-slate-300 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-700 dark:text-slate-200 transition-colors flex items-center space-x-2"
          >
            <span class={loading ? 'animate-spin' : ''}>🔄</span>
            <span>Refresh</span>
          </button>
        </div>
      </div>

      {/* Action Notification */}
      {actionSuccess && (
        <div aria-live="polite" class="p-3 bg-emerald-50 dark:bg-emerald-950/60 border border-emerald-300 dark:border-emerald-800 rounded-lg text-emerald-800 dark:text-emerald-300 text-sm flex items-center justify-between">
          <span>✓ {actionSuccess}</span>
          <button
            onClick={() => setActionSuccess(null)}
            class="text-xs text-emerald-600 dark:text-emerald-400 hover:underline"
          >
            Dismiss
          </button>
        </div>
      )}

      {/* Global Error */}
      {error && (
        <div role="alert" class="p-4 bg-red-50 dark:bg-red-950/60 border border-red-300 dark:border-red-800 rounded-lg text-red-800 dark:text-red-300 text-sm">
          ⚠️ {error}
        </div>
      )}

      {/* Search and Filters Bar */}
      <div class="bg-white dark:bg-slate-800 rounded-xl p-4 shadow-sm border border-slate-200 dark:border-slate-700 flex flex-col sm:flex-row gap-3 items-center justify-between">
        <div class="relative w-full sm:w-96">
          <span class="absolute inset-y-0 left-0 flex items-center pl-3 pointer-events-none text-slate-400" aria-hidden="true">
            🔍
          </span>
          <input
            type="text"
            value={searchTerm}
            onInput={(e) => setSearchTerm((e.target as HTMLInputElement).value)}
            placeholder="Search by ID, source or destination EID..."
            aria-label="Search by ID, source or destination EID"
            class="w-full pl-10 pr-4 py-2 rounded-lg border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 text-slate-900 dark:text-white text-sm focus:outline-none focus:ring-2 focus:ring-emerald-500"
          />
        </div>

        <div class="flex items-center space-x-3 w-full sm:w-auto justify-end">
          <label for="bundle-status-filter" class="text-xs font-medium text-slate-500 dark:text-slate-400">Status:</label>
          <select
            id="bundle-status-filter"
            value={statusFilter}
            onChange={(e) => setStatusFilter((e.target as HTMLSelectElement).value)}
            class="px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 text-slate-900 dark:text-white text-sm focus:outline-none focus:ring-2 focus:ring-emerald-500"
          >
            <option value="ALL">All Statuses</option>
            <option value="NEW">New</option>
            <option value="WAITING">Waiting</option>
            <option value="FORWARDPENDING">ForwardPending</option>
            <option value="WAITINGFORSERVICE">WaitingForService</option>
            <option value="COMPLETED">Completed</option>
            <option value="STORED">Stored</option>
          </select>

          <span class="text-xs text-slate-500 dark:text-slate-400 font-mono">
            {filteredBundles.length} / {bundles.length} bundles
          </span>
        </div>
      </div>

      {/* Bundles Table */}
      <div class="bg-white dark:bg-slate-800 rounded-xl shadow-sm border border-slate-200 dark:border-slate-700 overflow-hidden">
        {loading && bundles.length === 0 ? (
          <div class="flex items-center justify-center p-16">
            <div class="animate-spin rounded-full h-10 w-10 border-b-2 border-emerald-600"></div>
          </div>
        ) : filteredBundles.length === 0 ? (
          <div class="p-16 text-center text-slate-500 dark:text-slate-400">
            <div class="text-4xl mb-3">📭</div>
            <div class="text-base font-medium">No bundles found</div>
            <div class="text-xs text-slate-400 dark:text-slate-500 mt-1">
              {bundles.length === 0
                ? 'BPA storage currently contains no bundles.'
                : 'No bundles matched your current search filters.'}
            </div>
          </div>
        ) : (
          <div class="overflow-x-auto">
            <table class="w-full text-left text-sm">
              <thead class="bg-slate-50 dark:bg-slate-900/50 text-xs font-semibold uppercase tracking-wider text-slate-600 dark:text-slate-400 border-b border-slate-200 dark:border-slate-700">
                <tr>
                  <th class="px-3 py-3.5 whitespace-nowrap">Status</th>
                  <th class="px-3 py-3.5 whitespace-nowrap">Auth</th>
                  <th class="px-3 py-3.5 whitespace-nowrap">Bundle ID</th>
                  <th class="px-3 py-3.5">Source</th>
                  <th class="px-3 py-3.5">Destination</th>
                  <th class="px-3 py-3.5 whitespace-nowrap">Size</th>
                  <th class="px-3 py-3.5 whitespace-nowrap">Lifetime</th>
                  <th class="px-3 py-3.5 whitespace-nowrap">Created At</th>
                  <th class="px-3 py-3.5 text-right whitespace-nowrap">Actions</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-slate-200 dark:divide-slate-700">
                {filteredBundles.map((b) => (
                  <tr
                    key={b.id}
                    class="hover:bg-slate-50/80 dark:hover:bg-slate-700/30 transition-colors"
                  >
                    <td class="px-3 py-3 whitespace-nowrap">
                      <span
                        class={`inline-block px-2 py-0.5 text-xs font-semibold rounded-full border ${getStatusBadge(
                          b.status
                        )}`}
                      >
                        {b.status}
                      </span>
                    </td>
                    <td class="px-3 py-3 whitespace-nowrap">
                      <SecurityBadge status={b.auth_status} showLabel={false} />
                    </td>
                    <td class="px-3 py-3 font-mono text-xs text-slate-900 dark:text-slate-200 whitespace-nowrap">
                      <div class="inline-flex items-center space-x-1.5" title={b.id}>
                        <span
                          class="font-medium hover:text-emerald-600 dark:hover:text-emerald-400 cursor-pointer select-all transition-colors"
                          onClick={() => copyToClipboard(b.id)}
                          title={`Click to copy: ${b.id}`}
                        >
                          {formatShortId(b.id)}
                        </span>
                        <button
                          onClick={() => copyToClipboard(b.id)}
                          title="Copy Full Bundle ID"
                          class="p-0.5 hover:text-emerald-500 text-slate-400 transition-colors flex-shrink-0"
                        >
                          📋
                        </button>
                      </div>
                    </td>
                    <td class="px-3 py-3 font-mono text-xs text-slate-600 dark:text-slate-300 max-w-[130px] truncate" title={b.source}>
                      {b.source}
                    </td>
                    <td class="px-3 py-3 font-mono text-xs text-slate-600 dark:text-slate-300 max-w-[130px] truncate" title={b.destination}>
                      {b.destination}
                    </td>
                    <td class="px-3 py-3 whitespace-nowrap text-xs text-slate-600 dark:text-slate-300">
                      {formatBytes(b.size)}
                    </td>
                    <td class="px-3 py-3 whitespace-nowrap text-xs text-slate-600 dark:text-slate-300">
                      {formatLifetime(b.lifetime_secs)}
                    </td>
                    <td class="px-3 py-3 whitespace-nowrap text-xs text-slate-500 dark:text-slate-400" title={b.created_at}>
                      {b.created_at.length > 19 ? b.created_at.substring(0, 19).replace('T', ' ') : b.created_at}
                    </td>
                    <td class="px-3 py-3 whitespace-nowrap text-right space-x-1">
                      {/* Inspect button */}
                      <button
                        onClick={() => handleInspect(b.id)}
                        title="Inspect bundle details and payload"
                        class="px-2.5 py-1 text-xs font-medium rounded border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 transition-colors"
                      >
                        🔍 Inspect
                      </button>

                      {/* Download payload link */}
                      <a
                        href={`/api/v1/bundles/payload?id=${encodeURIComponent(b.id)}`}
                        download
                        title="Download raw payload"
                        class="inline-block px-2.5 py-1 text-xs font-medium rounded border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 transition-colors"
                      >
                        ⬇️
                      </a>

                      {/* Delete button */}
                      <button
                        onClick={() => setBundleToDelete(b)}
                        title="Delete bundle from storage"
                        class="px-2.5 py-1 text-xs font-medium rounded border border-red-200 dark:border-red-900/60 hover:bg-red-50 dark:hover:bg-red-950/50 text-red-600 dark:text-red-400 transition-colors"
                      >
                        🗑️
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Inspector Modal */}
      {selectedBundleId && (
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby="bundle-inspector-title"
          class="fixed inset-0 z-50 overflow-y-auto bg-slate-900/60 backdrop-blur-sm flex items-center justify-center p-4"
        >
          <div class="bg-white dark:bg-slate-800 rounded-2xl shadow-xl border border-slate-200 dark:border-slate-700 max-w-4xl w-full max-h-[90vh] flex flex-col overflow-hidden animate-in fade-in duration-150">
            {/* Modal Header */}
            <div class="px-6 py-4 border-b border-slate-200 dark:border-slate-700 flex items-center justify-between bg-slate-50 dark:bg-slate-900/50">
              <div class="flex items-center space-x-3">
                <span class="text-2xl" aria-hidden="true">📦</span>
                <div>
                  <h3 id="bundle-inspector-title" class="text-lg font-bold text-slate-900 dark:text-white">Bundle Inspector</h3>
                  <div class="text-xs font-mono text-slate-500 dark:text-slate-400 truncate max-w-md">
                    {selectedBundleId}
                  </div>
                </div>
              </div>
              <button
                onClick={() => {
                  setSelectedBundleId(null);
                  setBundleDetail(null);
                }}
                aria-label="Close modal"
                class="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 text-xl font-bold p-1 rounded-lg"
              >
                ✕
              </button>
            </div>

            {/* Modal Content */}
            <div class="p-6 overflow-y-auto flex-1 space-y-6">
              {loadingDetail ? (
                <div class="flex items-center justify-center p-12">
                  <div class="animate-spin rounded-full h-10 w-10 border-b-2 border-emerald-600"></div>
                </div>
              ) : detailError ? (
                <div class="p-4 bg-red-50 dark:bg-red-950/60 border border-red-300 dark:border-red-800 rounded-lg text-red-800 dark:text-red-300 text-sm">
                  ⚠️ {detailError}
                </div>
              ) : bundleDetail ? (
                <div class="space-y-6">
                  {/* Metadata Cards Grid */}
                  <div class="grid grid-cols-2 sm:grid-cols-4 gap-3">
                    <div class="p-3 bg-slate-50 dark:bg-slate-900/60 rounded-xl border border-slate-200 dark:border-slate-700">
                      <div class="text-xs font-semibold uppercase text-slate-500 dark:text-slate-400">Status</div>
                      <div class="mt-1">
                        <span class={`inline-block px-2 py-0.5 text-xs font-semibold rounded-full border ${getStatusBadge(bundleDetail.status)}`}>
                          {bundleDetail.status}
                        </span>
                      </div>
                    </div>

                    <div class="p-3 bg-slate-50 dark:bg-slate-900/60 rounded-xl border border-slate-200 dark:border-slate-700">
                      <div class="text-xs font-semibold uppercase text-slate-500 dark:text-slate-400">MIME Type</div>
                      <div class="mt-1">
                        <span class={`inline-block px-2 py-0.5 text-xs font-semibold rounded-full border ${getMimeBadge(bundleDetail.mime_type)}`}>
                          {bundleDetail.mime_type}
                        </span>
                      </div>
                    </div>

                    <div class="p-3 bg-slate-50 dark:bg-slate-900/60 rounded-xl border border-slate-200 dark:border-slate-700">
                      <div class="text-xs font-semibold uppercase text-slate-500 dark:text-slate-400">Payload Size</div>
                      <div class="mt-1 font-semibold text-slate-900 dark:text-white text-sm">
                        {formatBytes(bundleDetail.payload_size)}
                      </div>
                    </div>

                    <div class="p-3 bg-slate-50 dark:bg-slate-900/60 rounded-xl border border-slate-200 dark:border-slate-700">
                      <div class="text-xs font-semibold uppercase text-slate-500 dark:text-slate-400">Total Size</div>
                      <div class="mt-1 font-semibold text-slate-900 dark:text-white text-sm">
                        {formatBytes(bundleDetail.size)}
                      </div>
                    </div>
                  </div>

                  {/* EID & Timestamp Information */}
                  <div class="p-4 bg-slate-50 dark:bg-slate-900/60 rounded-xl border border-slate-200 dark:border-slate-700 space-y-2 text-xs">
                    <div class="flex justify-between py-1 border-b border-slate-200/60 dark:border-slate-800">
                      <span class="text-slate-500 dark:text-slate-400 font-medium">Source EID:</span>
                      <span class="font-mono text-slate-900 dark:text-slate-200">{bundleDetail.source}</span>
                    </div>
                    <div class="flex justify-between py-1 border-b border-slate-200/60 dark:border-slate-800">
                      <span class="text-slate-500 dark:text-slate-400 font-medium">Destination EID:</span>
                      <span class="font-mono text-slate-900 dark:text-slate-200">{bundleDetail.destination}</span>
                    </div>
                    <div class="flex justify-between py-1 border-b border-slate-200/60 dark:border-slate-800">
                      <span class="text-slate-500 dark:text-slate-400 font-medium">Created:</span>
                      <span class="text-slate-900 dark:text-slate-200">{bundleDetail.created_at}</span>
                    </div>
                    <div class="flex justify-between py-1 border-b border-slate-200/60 dark:border-slate-800">
                      <span class="text-slate-500 dark:text-slate-400 font-medium">BPSec BIB (RFC 9103):</span>
                      <SecurityBadge status={bundleDetail.auth_status} showLabel={true} />
                    </div>
                    <div class="flex justify-between py-1 border-b border-slate-200/60 dark:border-slate-800">
                      <span class="text-slate-500 dark:text-slate-400 font-medium">Lifetime:</span>
                      <span class="text-slate-900 dark:text-slate-200">{formatLifetime(bundleDetail.lifetime_secs)} ({bundleDetail.lifetime_secs}s)</span>
                    </div>
                    {bundleDetail.storage_name && (
                      <div class="flex justify-between py-1">
                        <span class="text-slate-500 dark:text-slate-400 font-medium">Disk File:</span>
                        <span class="font-mono text-slate-900 dark:text-slate-200">{bundleDetail.storage_name}</span>
                      </div>
                    )}
                  </div>

                  {/* Multi-Format Payload Viewer */}
                  <div>
                    <div class="flex items-center justify-between mb-2">
                      <div class="text-sm font-bold text-slate-900 dark:text-white flex items-center gap-2">
                        <span>📄</span>
                        <span>Payload Content</span>
                      </div>

                      {/* View Switcher */}
                      <div class="flex items-center space-x-1 bg-slate-100 dark:bg-slate-900 p-1 rounded-lg border border-slate-200 dark:border-slate-700" role="group" aria-label="Payload view mode">
                        <button
                          onClick={() => setDetailViewMode('preview')}
                          aria-pressed={detailViewMode === 'preview'}
                          class={`px-3 py-1 text-xs font-medium rounded transition-colors ${
                            detailViewMode === 'preview'
                              ? 'bg-white dark:bg-slate-800 text-slate-900 dark:text-white shadow-xs'
                              : 'text-slate-500 hover:text-slate-900 dark:hover:text-white'
                          }`}
                        >
                          Preview
                        </button>
                        <button
                          onClick={() => setDetailViewMode('hex')}
                          aria-pressed={detailViewMode === 'hex'}
                          class={`px-3 py-1 text-xs font-medium rounded transition-colors ${
                            detailViewMode === 'hex'
                              ? 'bg-white dark:bg-slate-800 text-slate-900 dark:text-white shadow-xs'
                              : 'text-slate-500 hover:text-slate-900 dark:hover:text-white'
                          }`}
                        >
                          Hex Dump
                        </button>
                        {!bundleDetail.is_binary && (
                          <button
                            onClick={() => setDetailViewMode('raw')}
                            aria-pressed={detailViewMode === 'raw'}
                            class={`px-3 py-1 text-xs font-medium rounded transition-colors ${
                              detailViewMode === 'raw'
                                ? 'bg-white dark:bg-slate-800 text-slate-900 dark:text-white shadow-xs'
                                : 'text-slate-500 hover:text-slate-900 dark:hover:text-white'
                            }`}
                          >
                            Raw Text
                          </button>
                        )}
                      </div>
                    </div>

                    {/* Mode: Preview */}
                    {detailViewMode === 'preview' && (
                      <div class="bg-slate-50 dark:bg-slate-900/90 rounded-xl p-4 border border-slate-200 dark:border-slate-700 overflow-hidden">
                        {bundleDetail.mime_type.startsWith('image/') ? (
                          <div class="flex flex-col items-center justify-center p-4">
                            <img
                              src={`/api/v1/bundles/payload?id=${encodeURIComponent(bundleDetail.id)}`}
                              alt="Bundle Image Payload"
                              class="max-h-96 max-w-full rounded-lg shadow-md border border-slate-200 dark:border-slate-700 object-contain bg-white dark:bg-slate-950 p-2"
                            />
                            <div class="text-xs text-slate-500 dark:text-slate-400 mt-2 font-mono">
                              {bundleDetail.mime_type} · {formatBytes(bundleDetail.payload_size)}
                            </div>
                          </div>
                        ) : bundleDetail.mime_type.startsWith('video/') ? (
                          <div class="flex flex-col items-center justify-center p-4 space-y-3">
                            <video
                              controls
                              playsinline
                              preload="metadata"
                              class="max-h-96 max-w-full rounded-lg shadow-md border border-slate-200 dark:border-slate-700 bg-black"
                            >
                              <source
                                src={`/api/v1/bundles/payload?id=${encodeURIComponent(bundleDetail.id)}`}
                                type={bundleDetail.mime_type}
                              />
                              Your browser does not support the video element.
                            </video>
                            <div class="text-xs text-slate-500 dark:text-slate-400 font-mono">
                              {bundleDetail.mime_type} · {formatBytes(bundleDetail.payload_size)}
                            </div>
                          </div>
                        ) : bundleDetail.mime_type.startsWith('audio/') ? (
                          <div class="flex flex-col items-center justify-center p-6 space-y-4">
                            <div class="text-4xl">🎙️</div>
                            <audio
                              controls
                              class="w-full max-w-md"
                            >
                              <source
                                src={`/api/v1/bundles/payload?id=${encodeURIComponent(bundleDetail.id)}`}
                                type="audio/ogg; codecs=opus"
                              />
                              <source
                                src={`/api/v1/bundles/payload?id=${encodeURIComponent(bundleDetail.id)}`}
                                type="audio/ogg"
                              />
                              Your browser does not support the audio element.
                            </audio>
                            <div class="text-xs text-slate-500 dark:text-slate-400 font-mono">
                              {bundleDetail.mime_type} · {formatBytes(bundleDetail.payload_size)}
                            </div>
                          </div>
                        ) : bundleDetail.mime_type.includes('senml') || bundleDetail.mime_type.includes('json') ? (
                          <div class="space-y-2">
                            <div class="flex items-center justify-between text-xs text-slate-500 dark:text-slate-400 font-mono">
                              <span>Structured JSON / SenML Sensor Record</span>
                              <button
                                onClick={() => copyToClipboard(bundleDetail.payload_preview || '')}
                                class="hover:text-emerald-500"
                              >
                                📋 Copy JSON
                              </button>
                            </div>
                            <pre class="font-mono text-xs text-slate-900 dark:text-slate-200 bg-slate-100 dark:bg-slate-950 p-4 rounded-lg overflow-x-auto max-h-96">
                              {(() => {
                                try {
                                  return JSON.stringify(JSON.parse(bundleDetail.payload_preview || '{}'), null, 2);
                                } catch {
                                  return bundleDetail.payload_preview;
                                }
                              })()}
                            </pre>
                          </div>
                        ) : !bundleDetail.is_binary ? (
                          <div class="space-y-2">
                            <div class="flex items-center justify-between text-xs text-slate-500 dark:text-slate-400 font-mono">
                              <span>{bundleDetail.mime_type === 'text/markdown' ? 'Markdown Document' : 'Plain Text'}</span>
                              <button
                                onClick={() => copyToClipboard(bundleDetail.payload_preview || '')}
                                class="hover:text-emerald-500"
                              >
                                📋 Copy Text
                              </button>
                            </div>
                            {bundleDetail.mime_type === 'text/markdown' ? (
                              <div
                                class="prose prose-sm dark:prose-invert max-w-none p-4 bg-slate-100 dark:bg-slate-950 rounded-lg overflow-y-auto max-h-96"
                                dangerouslySetInnerHTML={{
                                  __html: renderMarkdown(bundleDetail.payload_preview || '', { allowImages: true }),
                                }}
                              />
                            ) : (
                              <pre class="font-sans text-sm text-slate-900 dark:text-slate-200 whitespace-pre-wrap break-words bg-slate-100 dark:bg-slate-950 p-4 rounded-lg overflow-y-auto max-h-96">
                                {bundleDetail.payload_preview}
                              </pre>
                            )}
                          </div>
                        ) : (
                          <div class="space-y-2">
                            <div class="text-xs text-slate-500 dark:text-slate-400 font-mono">
                              Binary payload detected ({bundleDetail.mime_type})
                            </div>
                            <pre class="font-mono text-xs text-slate-800 dark:text-slate-300 bg-slate-100 dark:bg-slate-950 p-3 rounded-lg overflow-x-auto max-h-96">
                              {bundleDetail.hex_dump || 'No hex dump available'}
                            </pre>
                          </div>
                        )}
                      </div>
                    )}

                    {/* Mode: Hex Dump */}
                    {detailViewMode === 'hex' && (
                      <div class="bg-slate-950 rounded-xl p-4 border border-slate-800 overflow-hidden">
                        <div class="flex items-center justify-between text-xs text-slate-400 font-mono mb-2">
                          <span>Offset (h) -- 00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F -- ASCII</span>
                          <button
                            onClick={() => copyToClipboard(bundleDetail.hex_dump || '')}
                            class="hover:text-emerald-400"
                          >
                            📋 Copy Hex
                          </button>
                        </div>
                        <pre class="font-mono text-xs text-emerald-400 bg-slate-950 overflow-x-auto max-h-96 select-text">
                          {bundleDetail.hex_dump || 'No payload data to format'}
                        </pre>
                      </div>
                    )}

                    {/* Mode: Raw Text */}
                    {detailViewMode === 'raw' && !bundleDetail.is_binary && (
                      <div class="bg-slate-100 dark:bg-slate-950 rounded-xl p-4 border border-slate-200 dark:border-slate-800">
                        <textarea
                          readOnly
                          value={bundleDetail.payload_preview || ''}
                          rows={12}
                          class="w-full font-mono text-xs bg-transparent text-slate-900 dark:text-slate-200 border-none resize-none focus:outline-none"
                        />
                      </div>
                    )}
                  </div>
                </div>
              ) : null}
            </div>

            {/* Modal Footer Actions */}
            <div class="px-6 py-4 border-t border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/50 flex items-center justify-between">
              <div>
                {bundleDetail && (
                  <button
                    onClick={() => setBundleToDelete(bundleDetail)}
                    class="px-3 py-1.5 text-xs font-medium rounded-lg text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-950/50 border border-red-300 dark:border-red-900/60 transition-colors"
                  >
                    🗑️ Delete from Storage
                  </button>
                )}
              </div>
              <div class="flex items-center space-x-3">
                {bundleDetail && (
                  <a
                    href={`/api/v1/bundles/payload?id=${encodeURIComponent(bundleDetail.id)}`}
                    download
                    class="px-4 py-2 text-xs font-semibold rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white shadow-sm transition-colors flex items-center space-x-2"
                  >
                    <span>⬇️</span>
                    <span>Download Payload</span>
                  </a>
                )}
                <button
                  onClick={() => {
                    setSelectedBundleId(null);
                    setBundleDetail(null);
                  }}
                  class="px-4 py-2 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-600 hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-700 dark:text-slate-200 transition-colors"
                >
                  Close
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Delete Confirmation Modal */}
      {bundleToDelete && (
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby="delete-bundle-title"
          class="fixed inset-0 z-50 overflow-y-auto bg-slate-900/60 backdrop-blur-sm flex items-center justify-center p-4"
        >
          <div class="bg-white dark:bg-slate-800 rounded-2xl shadow-xl border border-slate-200 dark:border-slate-700 max-w-md w-full p-6 space-y-4 animate-in fade-in duration-150">
            <div class="flex items-center space-x-3 text-red-600 dark:text-red-400">
              <span class="text-3xl" aria-hidden="true">⚠️</span>
              <h3 id="delete-bundle-title" class="text-lg font-bold text-slate-900 dark:text-white">Delete Bundle?</h3>
            </div>

            <p class="text-sm text-slate-600 dark:text-slate-300">
              Are you sure you want to delete this bundle from BPA storage?
            </p>

            <div class="p-3 bg-slate-100 dark:bg-slate-900 rounded-lg text-xs font-mono text-slate-800 dark:text-slate-200 break-all border border-slate-200 dark:border-slate-700">
              {bundleToDelete.id}
            </div>

            <p class="text-xs text-amber-600 dark:text-amber-400 font-medium">
              This action will permanently delete the bundle metadata row from SQLite and remove the payload data file from disk.
            </p>

            <div class="flex items-center justify-end space-x-3 pt-2">
              <button
                onClick={() => setBundleToDelete(null)}
                disabled={deleting}
                class="px-4 py-2 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-600 text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-700 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleConfirmDelete}
                disabled={deleting}
                class="px-4 py-2 text-xs font-semibold rounded-lg bg-red-600 hover:bg-red-500 text-white shadow-sm transition-colors flex items-center space-x-2"
              >
                {deleting && <span class="animate-spin">🔄</span>}
                <span>{deleting ? 'Deleting...' : 'Confirm Delete'}</span>
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
