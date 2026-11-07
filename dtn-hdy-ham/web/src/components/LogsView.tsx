import { h } from 'preact';
import { useState, useEffect, useRef } from 'preact/hooks';

export interface LogRecord {
  id: number;
  timestamp: string;
  level: string;
  target: string;
  message: string;
}

interface LogsViewProps {
  incomingLog: LogRecord | null;
  sseStatus: 'connected' | 'disconnected' | 'connecting';
}

export function LogsView({ incomingLog, sseStatus }: LogsViewProps) {
  const [logs, setLogs] = useState<LogRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [levelFilter, setLevelFilter] = useState<'ALL' | 'INFO' | 'WARN' | 'ERROR' | 'DEBUG' | 'TRACE'>('ALL');
  const [searchQuery, setSearchQuery] = useState('');
  const [autoScroll, setAutoScroll] = useState(true);

  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const isAutoScrollingRef = useRef(false);

  // Initial fetch of buffered logs
  useEffect(() => {
    let mounted = true;
    fetch('/api/v1/logs')
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        return res.json();
      })
      .then((data) => {
        if (mounted && Array.isArray(data.logs)) {
          setLogs(data.logs);
          setLoading(false);
        }
      })
      .catch((err) => {
        if (mounted) {
          setError(`Failed to load log history: ${err.message}`);
          setLoading(false);
        }
      });
    return () => {
      mounted = false;
    };
  }, []);

  // Handle incoming live log record from parent SSE
  useEffect(() => {
    if (!incomingLog) return;
    setLogs((prev) => {
      // Prevent duplicate append if ID already exists
      if (prev.length > 0 && prev[prev.length - 1].id === incomingLog.id) {
        return prev;
      }
      const updated = [...prev, incomingLog];
      // Keep maximum 1000 logs in DOM memory to avoid performance degradation
      if (updated.length > 1000) {
        return updated.slice(updated.length - 1000);
      }
      return updated;
    });
  }, [incomingLog]);

  // Handle auto-scroll sticking to bottom
  useEffect(() => {
    if (autoScroll && scrollContainerRef.current) {
      isAutoScrollingRef.current = true;
      scrollContainerRef.current.scrollTop = scrollContainerRef.current.scrollHeight;
      setTimeout(() => {
        isAutoScrollingRef.current = false;
      }, 50);
    }
  }, [logs, autoScroll]);

  const handleScroll = () => {
    if (isAutoScrollingRef.current || !scrollContainerRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = scrollContainerRef.current;
    const distanceToBottom = scrollHeight - scrollTop - clientHeight;
    // If scrolled more than 40px away from bottom, pause auto-scroll
    if (distanceToBottom > 40) {
      if (autoScroll) setAutoScroll(false);
    } else {
      if (!autoScroll) setAutoScroll(true);
    }
  };

  const handleClear = () => {
    setLogs([]);
  };

  const handleExport = () => {
    const lines = filteredLogs.map(
      (l) => `[${l.timestamp}] [${l.level.padEnd(5)}] [${l.target}] ${l.message}`
    );
    const content = lines.join('\n');
    const blob = new Blob([content], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `dtn-logs-${new Date().toISOString().replace(/[:.]/g, '-')}.txt`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const filteredLogs = logs.filter((log) => {
    const levelMatch =
      levelFilter === 'ALL' || log.level.toUpperCase() === levelFilter;
    if (!levelMatch) return false;

    if (!searchQuery.trim()) return true;
    const q = searchQuery.toLowerCase();
    return (
      log.message.toLowerCase().includes(q) ||
      log.target.toLowerCase().includes(q) ||
      log.level.toLowerCase().includes(q)
    );
  });

  const getLevelBadgeClass = (level: string) => {
    switch (level.toUpperCase()) {
      case 'ERROR':
        return 'bg-rose-100 text-rose-800 dark:bg-rose-950/80 dark:text-rose-300 border-rose-300 dark:border-rose-800';
      case 'WARN':
        return 'bg-amber-100 text-amber-800 dark:bg-amber-950/80 dark:text-amber-300 border-amber-300 dark:border-amber-800';
      case 'INFO':
        return 'bg-sky-100 text-sky-800 dark:bg-sky-950/80 dark:text-sky-300 border-sky-300 dark:border-sky-800';
      case 'DEBUG':
        return 'bg-purple-100 text-purple-800 dark:bg-purple-950/80 dark:text-purple-300 border-purple-300 dark:border-purple-800';
      case 'TRACE':
      default:
        return 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300 border-slate-300 dark:border-slate-700';
    }
  };

  const formatTimestamp = (raw: string) => {
    try {
      const d = new Date(raw);
      if (isNaN(d.getTime())) return raw;
      return d.toLocaleTimeString([], {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        fractionalSecondDigits: 3,
      });
    } catch {
      return raw;
    }
  };

  return (
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6 space-y-4">
      {/* Header and Controls */}
      <div class="bg-white dark:bg-slate-800 rounded-xl shadow-sm border border-slate-200 dark:border-slate-700 p-4">
        <div class="flex flex-col md:flex-row md:items-center md:justify-between gap-4">
          <div>
            <div class="flex items-center space-x-3">
              <h1 class="text-xl font-bold text-slate-900 dark:text-white flex items-center gap-2">
                <span>📟</span> System Log Console
              </h1>
              <div
                class={`flex items-center space-x-1.5 px-2.5 py-0.5 rounded-full text-xs font-semibold border ${
                  sseStatus === 'connected'
                    ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300 border-emerald-300 dark:border-emerald-800'
                    : sseStatus === 'connecting'
                    ? 'bg-amber-50 text-amber-700 dark:bg-amber-950/60 dark:text-amber-300 border-amber-300 dark:border-amber-800'
                    : 'bg-rose-50 text-rose-700 dark:bg-rose-950/60 dark:text-rose-300 border-rose-300 dark:border-rose-800'
                }`}
                title={`SSE stream is ${sseStatus}`}
              >
                <span
                  class={`w-2 h-2 rounded-full ${
                    sseStatus === 'connected'
                      ? 'bg-emerald-500 animate-pulse'
                      : sseStatus === 'connecting'
                      ? 'bg-amber-500 animate-ping'
                      : 'bg-rose-500'
                  }`}
                />
                <span class="capitalize">{sseStatus === 'connected' ? 'Live Stream' : sseStatus}</span>
              </div>
            </div>
            <p class="text-xs text-slate-500 dark:text-slate-400 mt-1">
              Real-time tracing records from daemon and protocol convergence layers.
            </p>
          </div>

          <div class="flex flex-wrap items-center gap-2">
            {/* Auto-scroll toggle */}
            <button
              onClick={() => {
                setAutoScroll(!autoScroll);
                if (!autoScroll && scrollContainerRef.current) {
                  scrollContainerRef.current.scrollTop = scrollContainerRef.current.scrollHeight;
                }
              }}
              class={`px-3 py-1.5 text-xs font-semibold rounded-lg border transition-colors flex items-center space-x-1.5 ${
                autoScroll
                  ? 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border-emerald-300 dark:border-emerald-800'
                  : 'bg-slate-100 dark:bg-slate-700 text-slate-600 dark:text-slate-300 border-slate-300 dark:border-slate-600'
              }`}
              title="Toggle automatic scrolling to bottom on incoming logs"
            >
              <span>{autoScroll ? '⬇️ Auto-scroll: ON' : '⏸️ Auto-scroll: PAUSED'}</span>
            </button>

            {/* Clear console */}
            <button
              onClick={handleClear}
              class="px-3 py-1.5 text-xs font-semibold rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 transition-colors"
              title="Clear visible logs from this tab"
            >
              🗑️ Clear
            </button>

            {/* Export logs */}
            <button
              onClick={handleExport}
              disabled={filteredLogs.length === 0}
              class="px-3 py-1.5 text-xs font-semibold rounded-lg bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white transition-colors flex items-center space-x-1.5 shadow-sm"
              title="Download displayed logs as text file"
            >
              <span>💾 Export (.txt)</span>
            </button>
          </div>
        </div>

        {/* Filter bar */}
        <div class="mt-4 pt-3 border-t border-slate-100 dark:border-slate-700/60 flex flex-col sm:flex-row gap-3 items-center justify-between">
          <div class="flex flex-wrap items-center gap-1.5 w-full sm:w-auto">
            <span class="text-xs font-semibold text-slate-500 dark:text-slate-400 mr-1">Level:</span>
            {(['ALL', 'INFO', 'WARN', 'ERROR', 'DEBUG', 'TRACE'] as const).map((lvl) => (
              <button
                key={lvl}
                onClick={() => setLevelFilter(lvl)}
                class={`px-2.5 py-1 text-xs font-mono font-semibold rounded-md border transition-colors ${
                  levelFilter === lvl
                    ? 'bg-emerald-600 text-white border-emerald-600 shadow-xs'
                    : 'bg-slate-50 dark:bg-slate-900/50 text-slate-600 dark:text-slate-400 border-slate-200 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800'
                }`}
              >
                {lvl}
              </button>
            ))}
          </div>

          <div class="flex items-center space-x-2 w-full sm:w-72">
            <input
              type="text"
              placeholder="Search in logs..."
              value={searchQuery}
              onInput={(e) => setSearchQuery((e.target as HTMLInputElement).value)}
              class="w-full px-3 py-1.5 text-xs rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900 text-slate-900 dark:text-white placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-emerald-500 font-mono"
            />
            {searchQuery && (
              <button
                onClick={() => setSearchQuery('')}
                class="text-xs text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 px-1"
                title="Clear search"
                aria-label="Clear search"
              >
                ✕
              </button>
            )}
          </div>
        </div>
      </div>

      {/* Log Terminal Window */}
      <div class="bg-slate-950 text-slate-100 rounded-xl shadow-lg border border-slate-800 flex flex-col font-mono text-xs overflow-hidden">
        {/* Terminal Header */}
        <div class="bg-slate-900/90 px-4 py-2 border-b border-slate-800 flex items-center justify-between select-none">
          <div class="flex items-center space-x-2">
            <div class="w-3 h-3 rounded-full bg-rose-500/80" />
            <div class="w-3 h-3 rounded-full bg-amber-500/80" />
            <div class="w-3 h-3 rounded-full bg-emerald-500/80" />
            <span class="text-slate-400 text-xs ml-2">dtn-hdy-ham daemon stream</span>
          </div>
          <div class="text-slate-500 text-xs">
            Showing {filteredLogs.length} / {logs.length} entries
          </div>
        </div>

        {/* Scrollable Container */}
        <div
          ref={scrollContainerRef}
          onScroll={handleScroll}
          class="h-[650px] overflow-y-auto p-4 space-y-1.5 select-text focus:outline-none"
          tabIndex={0}
          aria-label="Terminal Log Output"
        >
          {loading ? (
            <div class="h-full flex items-center justify-center text-slate-400 space-x-2">
              <span class="animate-spin">⏳</span>
              <span>Loading log history...</span>
            </div>
          ) : error ? (
            <div class="h-full flex items-center justify-center text-rose-400">
              <span>{error}</span>
            </div>
          ) : filteredLogs.length === 0 ? (
            <div class="h-full flex flex-col items-center justify-center text-slate-500 space-y-2">
              <span class="text-3xl">📭</span>
              <span>No log records match current filters</span>
              {searchQuery && (
                <button
                  onClick={() => setSearchQuery('')}
                  class="text-xs text-emerald-400 hover:underline"
                >
                  Clear search query
                </button>
              )}
            </div>
          ) : (
            filteredLogs.map((log) => (
              <div
                key={log.id}
                class="flex items-start space-x-2.5 hover:bg-slate-900/70 py-0.5 px-1.5 rounded transition-colors group"
              >
                <span class="text-slate-500 shrink-0 select-none">
                  {formatTimestamp(log.timestamp)}
                </span>
                <span
                  class={`px-1.5 py-0.2 rounded text-[10px] font-bold uppercase tracking-wider shrink-0 border select-none ${getLevelBadgeClass(
                    log.level
                  )}`}
                >
                  {log.level.padEnd(5)}
                </span>
                <span class="text-slate-400 shrink-0 font-semibold max-w-[200px] truncate" title={log.target}>
                  {log.target}:
                </span>
                <span class="text-slate-200 break-all whitespace-pre-wrap flex-1">
                  {log.message}
                </span>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
