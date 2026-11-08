import { h } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import { renderMarkdown } from '../utils/markdown';
import { SecurityBadge } from './SecurityBadge';

export interface FileBundleRecord {
  id: string;
  source_eid: string;
  destination_eid: string;
  creation_timestamp: number;
  lifetime_secs: number;
  expiry_timestamp: number;
  size: number;
  payload_path: string;
  mime_type: string;
  is_sticky: boolean;
  is_read: boolean;
  created_at: string;
  auth_status?: string;
}

export interface FileDetailResponse {
  bundle: FileBundleRecord;
  payload_preview?: string | null;
  file_missing: boolean;
}

interface FilesViewProps {
  stationNodeId?: string;
  stationCallsign?: string;
  selectedBundleId?: string | null;
  onSelectBundle?: (id: string | null) => void;
  onReply?: (sourceEid: string) => void;
  onUnreadChange?: (unread: number) => void;
}

// Format byte sizes into readable units
function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

// Format timestamp
function formatDate(isoStr: string | number): string {
  try {
    const d = typeof isoStr === 'number' ? new Date(isoStr) : new Date(isoStr);
    if (isNaN(d.getTime())) return '';
    return d.toLocaleString([], {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  } catch {
    return '';
  }
}

// Safely escape HTML characters
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;');
}

export function FilesView({
  stationNodeId = 'dtn://station/',
  stationCallsign,
  selectedBundleId,
  onSelectBundle,
  onReply,
  onUnreadChange,
}: FilesViewProps) {
  const filesEndpoint = `${(stationNodeId || (stationCallsign ? `dtn://${stationCallsign.toLowerCase()}/` : 'dtn://station/')).replace(/\/+$/, '')}/files`;
  const [bundles, setBundles] = useState<FileBundleRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [searchFilter, setSearchFilter] = useState('');

  // Selected bundle detail state
  const [detailData, setDetailData] = useState<FileDetailResponse | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [copiedId, setCopiedId] = useState(false);

  // Fetch list of file bundles
  const fetchBundles = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch('/api/v1/files');
      if (!res.ok) {
        throw new Error(`Failed to load file bundles: HTTP ${res.status}`);
      }
      const data = await res.json();
      if (Array.isArray(data)) {
        setBundles(data);

        const unread = data.filter((b) => !b.is_read).length;
        if (onUnreadChange) {
          onUnreadChange(unread);
        }
      }
    } catch (e: any) {
      setError(e.message || 'Error loading files');
    } finally {
      setLoading(false);
    }
  };

  // Fetch single bundle detail
  const fetchDetail = async (id: string) => {
    setDetailLoading(true);
    try {
      const res = await fetch(`/api/v1/files/${encodeURIComponent(id)}`);
      if (!res.ok) {
        throw new Error(`Bundle not found (HTTP ${res.status})`);
      }
      const data: FileDetailResponse = await res.json();
      setDetailData(data);

      // Update read status in local list
      setBundles((prev) =>
        prev.map((b) => (b.id === id ? { ...b, is_read: true } : b))
      );
      if (onUnreadChange) {
        setBundles((cur) => {
          const unread = cur.filter((b) => !b.is_read).length;
          onUnreadChange(unread);
          return cur;
        });
      }
    } catch (e: any) {
      setError(e.message);
    } finally {
      setDetailLoading(false);
    }
  };

  // Toggle sticky
  const handleToggleSticky = async (id: string, e?: Event) => {
    if (e) e.stopPropagation();
    try {
      const res = await fetch(`/api/v1/files/${encodeURIComponent(id)}/sticky`, {
        method: 'POST',
      });
      if (res.ok) {
        const json = await res.json();
        const nextSticky = json.is_sticky;

        setBundles((prev) =>
          prev.map((b) => (b.id === id ? { ...b, is_sticky: nextSticky } : b))
        );

        if (detailData && detailData.bundle.id === id) {
          setDetailData({
            ...detailData,
            bundle: { ...detailData.bundle, is_sticky: nextSticky },
          });
        }
      }
    } catch (err) {
      console.error('Failed to toggle sticky', err);
    }
  };

  // Delete bundle
  const handleDelete = async (id: string, e?: Event) => {
    if (e) e.stopPropagation();
    if (!window.confirm('Are you sure you want to delete this bundle and its payload file?')) {
      return;
    }

    try {
      const res = await fetch(`/api/v1/files/${encodeURIComponent(id)}`, {
        method: 'DELETE',
      });
      if (res.ok) {
        setBundles((prev) => prev.filter((b) => b.id !== id));
        if (selectedBundleId === id) {
          if (onSelectBundle) onSelectBundle(null);
        }
      }
    } catch (err) {
      console.error('Failed to delete file bundle', err);
    }
  };

  // Initial load
  useEffect(() => {
    fetchBundles();
  }, []);

  // When selectedBundleId prop changes
  useEffect(() => {
    if (selectedBundleId) {
      fetchDetail(selectedBundleId);
    } else {
      setDetailData(null);
    }
  }, [selectedBundleId]);

  // Copy bundle ID helper
  const copyBundleId = (id: string) => {
    navigator.clipboard.writeText(id);
    setCopiedId(true);
    setTimeout(() => setCopiedId(false), 2000);
  };

  // Filtered list
  const filtered = bundles.filter((b) => {
    const q = searchFilter.toLowerCase();
    return (
      b.source_eid.toLowerCase().includes(q) ||
      b.destination_eid.toLowerCase().includes(q) ||
      b.mime_type.toLowerCase().includes(q) ||
      b.id.toLowerCase().includes(q)
    );
  });

  return (
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
      {/* =========================================================================
          LEVEL 2: BUNDLE DETAILS VIEW
          ========================================================================= */}
      {selectedBundleId && detailData ? (
        <div class="space-y-6">
          {/* Top Bar with Back & Actions */}
          <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4 bg-white dark:bg-slate-800 p-4 sm:p-5 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700">
            <button
              onClick={() => {
                if (onSelectBundle) onSelectBundle(null);
              }}
              class="inline-flex items-center space-x-2 text-sm font-semibold text-emerald-600 hover:text-emerald-500 dark:text-emerald-400"
            >
              <span>←</span>
              <span>Back to Files List</span>
            </button>

            {/* Actions Toolbar */}
            <div class="flex items-center flex-wrap gap-2">
              {/* Toggle Sticky */}
              <button
                onClick={() => handleToggleSticky(detailData.bundle.id)}
                class={`px-3 py-1.5 text-xs font-semibold rounded-xl border flex items-center space-x-1.5 transition ${
                  detailData.bundle.is_sticky
                    ? 'bg-amber-50 dark:bg-amber-950/50 border-amber-300 dark:border-amber-700 text-amber-700 dark:text-amber-300'
                    : 'border-slate-200 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200'
                }`}
                title={detailData.bundle.is_sticky ? 'Unpin from station retention' : 'Pin to protect against lifetime expiration'}
              >
                <span>📌</span>
                <span>{detailData.bundle.is_sticky ? 'Sticky (Protected)' : 'Make Sticky'}</span>
              </button>

              {/* Download */}
              <a
                href={`/api/v1/files/${encodeURIComponent(detailData.bundle.id)}/payload?download=true`}
                download
                class="px-3 py-1.5 text-xs font-semibold rounded-xl bg-slate-100 hover:bg-slate-200 dark:bg-slate-700 dark:hover:bg-slate-600 text-slate-800 dark:text-white flex items-center space-x-1.5 transition"
              >
                <span>💾</span>
                <span>Download ({formatBytes(detailData.bundle.size)})</span>
              </a>

              {/* Reply */}
              {onReply && (
                <button
                  onClick={() => onReply(detailData.bundle.source_eid)}
                  class="px-3 py-1.5 text-xs font-semibold rounded-xl bg-emerald-600 hover:bg-emerald-500 text-white flex items-center space-x-1.5 shadow-sm transition"
                >
                  <span>✉️</span>
                  <span>Reply</span>
                </button>
              )}

              {/* Delete */}
              <button
                onClick={() => handleDelete(detailData.bundle.id)}
                class="px-3 py-1.5 text-xs font-semibold rounded-xl bg-red-50 hover:bg-red-100 dark:bg-red-950/40 text-red-600 dark:text-red-400 border border-red-200 dark:border-red-800 flex items-center space-x-1 transition"
              >
                <span>🗑️</span>
                <span>Delete</span>
              </button>
            </div>
          </div>

          {/* Missing file alert */}
          {detailData.file_missing && (
            <div class="p-3.5 rounded-xl bg-amber-50 dark:bg-amber-950/50 border border-amber-200 dark:border-amber-800 text-xs text-amber-700 dark:text-amber-300 flex items-center space-x-2">
              <span>⚠️</span>
              <span>The physical payload file is missing from station disk storage. Raw download and full rendering may be unavailable.</span>
            </div>
          )}

          {/* Bundle Headers Information Grid */}
          <div class="bg-white dark:bg-slate-800 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700 p-5">
            <h3 class="text-xs font-bold uppercase tracking-wider text-slate-400 mb-4 flex items-center space-x-1.5">
              <span>📋</span>
              <span>Bundle Headers</span>
            </h3>

            <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4 text-xs">
              <div>
                <span class="text-slate-400 block mb-0.5 font-medium">Bundle Key / ID</span>
                <div class="flex items-center space-x-1 font-mono text-slate-900 dark:text-white">
                  <span class="truncate">{detailData.bundle.id}</span>
                  <button
                    onClick={() => copyBundleId(detailData.bundle.id)}
                    class="p-1 text-slate-400 hover:text-slate-600 dark:hover:text-white rounded"
                    title="Copy full Bundle ID"
                  >
                    {copiedId ? '✓' : '📋'}
                  </button>
                </div>
              </div>

              <div>
                <span class="text-slate-400 block mb-0.5 font-medium">Source EID</span>
                <span class="font-mono text-emerald-600 dark:text-emerald-400 font-bold break-all">
                  {detailData.bundle.source_eid}
                </span>
              </div>

              <div>
                <span class="text-slate-400 block mb-0.5 font-medium">Destination EID</span>
                <span class="font-mono text-slate-800 dark:text-slate-200 break-all">
                  {detailData.bundle.destination_eid}
                </span>
              </div>

              <div>
                <span class="text-slate-400 block mb-0.5 font-medium">Creation Date</span>
                <span class="text-slate-800 dark:text-slate-200">
                  {formatDate(detailData.bundle.created_at)}
                </span>
              </div>

              <div>
                <span class="text-slate-400 block mb-0.5 font-medium">Lifetime / Expiry</span>
                <span class="text-slate-800 dark:text-slate-200">
                  {detailData.bundle.lifetime_secs}s (
                  {detailData.bundle.expiry_timestamp > 0 &&
                  detailData.bundle.expiry_timestamp <= Date.now() ? (
                    <span
                      class="text-amber-600 dark:text-amber-400 font-semibold cursor-help"
                      title="Retained under station storage quota"
                    >
                      ⏳ Expired on {formatDate(detailData.bundle.expiry_timestamp)}
                    </span>
                  ) : (
                    `expires: ${formatDate(detailData.bundle.expiry_timestamp)}`
                  )}
                  )
                </span>
              </div>

              <div>
                <span class="text-slate-400 block mb-0.5 font-medium">MIME Type & Size</span>
                <span class="inline-flex items-center space-x-2">
                  <span class="px-2 py-0.5 rounded-full bg-slate-100 dark:bg-slate-700 text-slate-800 dark:text-slate-200 font-semibold font-mono text-[10px]">
                    {detailData.bundle.mime_type}
                  </span>
                  <span class="text-slate-500 font-medium">
                    {formatBytes(detailData.bundle.size)}
                  </span>
                </span>
              </div>

              <div>
                <span class="text-slate-400 block mb-0.5 font-medium">Authentification BPSec</span>
                <SecurityBadge status={detailData.bundle.auth_status} showLabel={true} />
              </div>
            </div>
          </div>

          {/* Payload Content Viewer */}
          <div class="bg-white dark:bg-slate-800 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700 p-5">
            <h3 class="text-xs font-bold uppercase tracking-wider text-slate-400 mb-4 flex items-center space-x-1.5">
              <span>👁️</span>
              <span>Payload Content</span>
            </h3>

            {detailLoading ? (
              <div class="py-12 text-center text-slate-400 text-sm">
                Loading payload...
              </div>
            ) : detailData.bundle.mime_type.startsWith('image/') ? (
              /* Image Viewer */
              <div class="flex flex-col items-center justify-center p-4 bg-slate-50 dark:bg-slate-900 rounded-xl">
                <img
                  src={`/api/v1/files/${encodeURIComponent(detailData.bundle.id)}/payload`}
                  alt="Payload Content"
                  class="max-h-[500px] max-w-full rounded-lg shadow-sm object-contain"
                />
              </div>
            ) : detailData.bundle.mime_type.startsWith('video/') ? (
              /* Video Player */
              <div class="p-4 bg-slate-50 dark:bg-slate-900 rounded-xl flex flex-col items-center">
                <video
                  controls
                  playsinline
                  preload="metadata"
                  class="max-h-[500px] max-w-full rounded-lg shadow-sm bg-black"
                >
                  <source
                    src={`/api/v1/files/${encodeURIComponent(detailData.bundle.id)}/payload`}
                    type={detailData.bundle.mime_type}
                  />
                  Your browser does not support the video element.
                </video>
              </div>
            ) : detailData.bundle.mime_type.startsWith('audio/') ? (
              /* Audio Player */
              <div class="p-6 bg-slate-50 dark:bg-slate-900 rounded-xl flex flex-col items-center">
                <span class="text-3xl mb-3">🎵</span>
                <audio
                  controls
                  class="w-full max-w-md"
                >
                  <source
                    src={`/api/v1/files/${encodeURIComponent(detailData.bundle.id)}/payload`}
                    type="audio/ogg; codecs=opus"
                  />
                  <source
                    src={`/api/v1/files/${encodeURIComponent(detailData.bundle.id)}/payload`}
                    type="audio/ogg"
                  />
                  Your browser does not support the audio element.
                </audio>
              </div>
            ) : detailData.payload_preview ? (
              /* Markdown / Text Document Viewer */
              <div
                class="prose prose-sm dark:prose-invert max-w-none p-4 bg-slate-50/50 dark:bg-slate-900/50 rounded-xl border border-slate-100 dark:border-slate-800"
                dangerouslySetInnerHTML={{
                  __html: renderMarkdown(detailData.payload_preview, { allowImages: true }),
                }}
              />
            ) : (
              /* Fallback for other binary files */
              <div class="py-12 text-center text-slate-400">
                <span class="text-4xl block mb-2">📦</span>
                <p class="font-medium text-slate-600 dark:text-slate-300">
                  Binary payload ({formatBytes(detailData.bundle.size)})
                </p>
                <p class="text-xs text-slate-400 mt-1">
                  Click the Download button above to inspect or save this file.
                </p>
              </div>
            )}
          </div>
        </div>
      ) : (
        /* =========================================================================
            LEVEL 1: BUNDLE LIST VIEW
            ========================================================================= */
        <div class="space-y-6">
          {/* Header */}
          <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
            <div>
              <h1 class="text-2xl font-bold tracking-tight text-slate-900 dark:text-white flex items-center space-x-2">
                <span>📁</span>
                <span>File Bundles</span>
              </h1>
              <p class="text-sm text-slate-500 dark:text-slate-400">
                Inbox for files and documents delivered to endpoint{' '}
                <code class="px-1.5 py-0.5 rounded bg-slate-200 dark:bg-slate-700 font-mono text-xs text-emerald-700 dark:text-emerald-300">
                  {filesEndpoint}
                </code>
              </p>
            </div>

            <button
              onClick={fetchBundles}
              disabled={loading}
              class="inline-flex items-center space-x-1.5 px-3.5 py-2 text-sm font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 hover:bg-slate-50 dark:hover:bg-slate-700/60 transition shadow-sm"
            >
              <span>{loading ? '⏳' : '🔄'}</span>
              <span>Refresh</span>
            </button>
          </div>

          {/* Search Bar */}
          <div class="relative">
            <span class="absolute inset-y-0 left-0 pl-3.5 flex items-center text-slate-400 text-sm">
              🔍
            </span>
            <input
              type="text"
              placeholder="Search by source EID, MIME type or bundle ID..."
              aria-label="Search by source EID, MIME type or bundle ID"
              value={searchFilter}
              onInput={(e) => setSearchFilter((e.target as HTMLInputElement).value)}
              class="w-full pl-10 pr-4 py-2 text-sm bg-white dark:bg-slate-800 text-slate-900 dark:text-white rounded-xl border border-slate-200 dark:border-slate-700 focus:border-emerald-500 outline-none shadow-sm transition"
            />
          </div>

          {/* Error Banner */}
          {error && (
            <div role="alert" class="p-3.5 rounded-xl bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800 text-xs text-red-600 dark:text-red-400">
              {error}
            </div>
          )}

          {/* List of Bundles */}
          {loading && bundles.length === 0 ? (
            <div class="py-16 text-center text-slate-400">
              <span>Loading file bundles...</span>
            </div>
          ) : filtered.length === 0 ? (
            <div class="py-16 text-center text-slate-400 bg-white dark:bg-slate-800 rounded-2xl border border-slate-200 dark:border-slate-700 p-8 shadow-sm">
              <span class="text-4xl block mb-2">📭</span>
              <p class="font-medium text-slate-600 dark:text-slate-300">No file bundles received yet</p>
              <p class="text-xs text-slate-400 mt-1 max-w-sm mx-auto">
                Bundles addressed to this station's <code class="font-mono">/files</code> service will automatically appear here.
              </p>
            </div>
          ) : (
            <div class="grid grid-cols-1 gap-3.5">
              {filtered.map((b) => (
                <div
                  key={b.id}
                  role="button"
                  tabIndex={0}
                  onClick={() => {
                    if (onSelectBundle) onSelectBundle(b.id);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      if (onSelectBundle) onSelectBundle(b.id);
                    }
                  }}
                  class="cursor-pointer bg-white dark:bg-slate-800 hover:border-emerald-500/60 dark:hover:border-emerald-500/60 rounded-2xl p-4 sm:p-5 shadow-sm border border-slate-200 dark:border-slate-700 transition flex flex-col sm:flex-row sm:items-center justify-between gap-3 group focus:outline-none focus:ring-2 focus:ring-emerald-500"
                >
                  <div class="flex items-start space-x-3 min-w-0">
                    {/* Icon */}
                    <div class="w-10 h-10 rounded-xl bg-slate-100 dark:bg-slate-700 flex-shrink-0 flex items-center justify-center text-lg" aria-hidden="true">
                      {b.mime_type.startsWith('image/')
                        ? '🖼️'
                        : b.mime_type.startsWith('video/')
                        ? '🎬'
                        : b.mime_type.startsWith('audio/')
                        ? '🎵'
                        : b.mime_type.startsWith('text/')
                        ? '📄'
                        : '📦'}
                    </div>

                    <div class="min-w-0">
                      <div class="flex items-center space-x-2 mb-1 flex-wrap">
                        <span class="font-bold text-sm text-slate-900 dark:text-white font-mono truncate">
                          {b.source_eid}
                        </span>

                        <SecurityBadge status={b.auth_status} showLabel={false} />

                        {!b.is_read && (
                          <span class="px-2 py-0.5 text-[10px] font-bold rounded-full bg-purple-100 text-purple-800 dark:bg-purple-900/60 dark:text-purple-300">
                            NEW
                          </span>
                        )}

                        {b.is_sticky && (
                          <span class="px-2 py-0.5 text-[10px] font-bold rounded-full bg-amber-100 text-amber-800 dark:bg-amber-900/60 dark:text-amber-300">
                            📌 STICKY
                          </span>
                        )}

                        {b.expiry_timestamp > 0 && b.expiry_timestamp <= Date.now() && (
                          <span
                            class="px-2 py-0.5 text-[10px] font-bold rounded-full bg-slate-200 text-slate-700 dark:bg-slate-700 dark:text-slate-300 flex items-center space-x-1 cursor-help"
                            title={`Lifetime expired on ${formatDate(
                              b.expiry_timestamp
                            )} — retained under station storage quota`}
                          >
                            <span>⏳ EXPIRED</span>
                          </span>
                        )}
                      </div>

                      <div class="flex items-center space-x-3 text-xs text-slate-500 dark:text-slate-400">
                        <span>{formatDate(b.created_at)}</span>
                        <span>·</span>
                        <span>{formatBytes(b.size)}</span>
                        <span>·</span>
                        <span class="font-mono text-[11px] text-slate-500 dark:text-slate-400">
                          {b.mime_type}
                        </span>
                      </div>
                    </div>
                  </div>

                  {/* Actions on Card */}
                  <div class="flex items-center space-x-2 self-end sm:self-center">
                    {/* Toggle Sticky Button */}
                    <button
                      onClick={(e) => handleToggleSticky(b.id, e)}
                      class={`p-2 rounded-lg border transition ${
                        b.is_sticky
                          ? 'border-amber-300 dark:border-amber-700 text-amber-600 dark:text-amber-300 bg-amber-50 dark:bg-amber-950/40'
                          : 'border-transparent hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-400'
                      }`}
                      title={b.is_sticky ? 'Sticky (protected)' : 'Mark sticky'}
                      aria-label={b.is_sticky ? 'Unpin file' : 'Pin file'}
                    >
                      <span aria-hidden="true">📌</span>
                    </button>

                    {/* Download */}
                    <a
                      href={`/api/v1/files/${encodeURIComponent(b.id)}/payload?download=true`}
                      download
                      onClick={(e) => e.stopPropagation()}
                      class="p-2 text-slate-400 hover:text-slate-700 dark:hover:text-white hover:bg-slate-100 dark:hover:bg-slate-700 rounded-lg transition"
                      title="Download payload"
                      aria-label="Download payload"
                    >
                      <span aria-hidden="true">💾</span>
                    </a>

                    {/* Delete */}
                    <button
                      onClick={(e) => handleDelete(b.id, e)}
                      class="p-2 text-slate-400 hover:text-red-600 dark:hover:text-red-400 hover:bg-red-50 dark:hover:bg-red-950/40 rounded-lg transition"
                      title="Delete bundle"
                      aria-label="Delete bundle"
                    >
                      <span aria-hidden="true">🗑️</span>
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
