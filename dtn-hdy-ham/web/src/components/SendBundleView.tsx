import { h } from 'preact';
import { useState } from 'preact/hooks';

interface SendBundleViewProps {
  stationNodeId?: string;
  defaultDestination?: string;
}

interface SendResult {
  status: string;
  bundle_id: string;
  source: string;
  destination: string;
  payload_size: number;
  lifetime_secs: number;
}

export function SendBundleView({ stationNodeId, defaultDestination }: SendBundleViewProps) {
  const [destination, setDestination] = useState(defaultDestination || '');
  const [source, setSource] = useState('');
  const [lifetime, setLifetime] = useState('3600');
  const [mode, setMode] = useState<'text' | 'file'>('text');
  const [textPayload, setTextPayload] = useState('');
  const [selectedFile, setSelectedFile] = useState<File | null>(null);

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<SendResult | null>(null);
  const [copied, setCopied] = useState(false);

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    setError(null);
    setResult(null);

    if (!destination.trim()) {
      setError('Please provide a destination EID (e.g. dtn://statb/incoming)');
      return;
    }

    if (mode === 'text' && !textPayload) {
      setError('Please enter a text message payload');
      return;
    }

    if (mode === 'file' && !selectedFile) {
      setError('Please choose a file to transmit');
      return;
    }

    setLoading(true);

    try {
      if (mode === 'file' && selectedFile) {
        // Multipart send
        const formData = new FormData();
        formData.append('destination', destination.trim());
        if (source.trim()) {
          formData.append('source', source.trim());
        }
        if (lifetime.trim()) {
          formData.append('lifetime', lifetime.trim());
        }
        formData.append('file', selectedFile);

        const res = await fetch('/api/v1/bundles', {
          method: 'POST',
          body: formData,
        });

        const data = await res.json();
        if (!res.ok) {
          throw new Error(data.error || `HTTP ${res.status}`);
        }
        setResult(data);
      } else {
        // JSON send
        const res = await fetch('/api/v1/bundles', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            destination: destination.trim(),
            source: source.trim() || undefined,
            lifetime: parseInt(lifetime, 10) || 3600,
            payload: textPayload,
          }),
        });

        const data = await res.json();
        if (!res.ok) {
          throw new Error(data.error || `HTTP ${res.status}`);
        }
        setResult(data);
      }
    } catch (err: any) {
      setError(err.message || 'Unknown network error occurred');
    } finally {
      setLoading(false);
    }
  };

  const copyBundleId = () => {
    if (result?.bundle_id) {
      navigator.clipboard.writeText(result.bundle_id);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  return (
    <div class="max-w-3xl mx-auto py-6 px-4 sm:px-6">
      <div class="bg-white dark:bg-slate-800 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700 overflow-hidden">
        {/* Card Header */}
        <div class="p-6 border-b border-slate-100 dark:border-slate-700">
          <div class="flex items-center space-x-3">
            <div class="w-10 h-10 rounded-xl bg-emerald-100 dark:bg-emerald-900/60 text-emerald-700 dark:text-emerald-300 flex items-center justify-center text-xl">
              ✉️
            </div>
            <div>
              <h2 class="text-xl font-bold text-slate-900 dark:text-white">
                Generic Bundle Dispatcher
              </h2>
              <p class="text-sm text-slate-500 dark:text-slate-400">
                Compose text messages or transmit files across the amateur radio DTN network
              </p>
            </div>
          </div>
        </div>

        {/* Form Body */}
        <form onSubmit={handleSubmit} class="p-6 space-y-6">
          {/* Destination EID */}
          <div>
            <label class="block text-xs font-bold uppercase tracking-wider text-slate-600 dark:text-slate-300 mb-2">
              Destination Endpoint ID (EID) <span class="text-emerald-500">*</span>
            </label>
            <input
              type="text"
              value={destination}
              onInput={(e) => setDestination((e.target as HTMLInputElement).value)}
              placeholder="dtn://statb/incoming or ipn:2.1"
              required
              class="w-full px-4 py-2.5 rounded-xl border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 font-mono text-sm text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-emerald-500 transition"
            />
            <p class="mt-1.5 text-xs text-slate-400">
              Target DTN node and optional service endpoint. Trailing slash is automatically normalized.
            </p>
          </div>

          {/* Grid: Source EID & Lifetime */}
          <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
            <div>
              <label class="block text-xs font-bold uppercase tracking-wider text-slate-600 dark:text-slate-300 mb-2">
                Source EID (Optional)
              </label>
              <input
                type="text"
                value={source}
                onInput={(e) => setSource((e.target as HTMLInputElement).value)}
                placeholder={stationNodeId || 'Default (Station EID)'}
                class="w-full px-4 py-2.5 rounded-xl border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 font-mono text-sm text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-emerald-500 transition"
              />
              <p class="mt-1.5 text-xs text-slate-400">
                Leave empty to use station default: <span class="font-mono">{stationNodeId}</span>
              </p>
            </div>

            <div>
              <label class="block text-xs font-bold uppercase tracking-wider text-slate-600 dark:text-slate-300 mb-2">
                Bundle Lifetime (Seconds)
              </label>
              <input
                type="number"
                min="10"
                value={lifetime}
                onInput={(e) => setLifetime((e.target as HTMLInputElement).value)}
                class="w-full px-4 py-2.5 rounded-xl border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 font-mono text-sm text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-emerald-500 transition"
              />
              <p class="mt-1.5 text-xs text-slate-400">
                Default 3600 seconds (1 hour store-and-forward retention).
              </p>
            </div>
          </div>

          {/* Payload Mode Switcher */}
          <div>
            <label class="block text-xs font-bold uppercase tracking-wider text-slate-600 dark:text-slate-300 mb-2">
              Payload Type
            </label>
            <div class="inline-flex p-1 rounded-xl bg-slate-100 dark:bg-slate-900 border border-slate-200 dark:border-slate-700">
              <button
                type="button"
                onClick={() => setMode('text')}
                class={`px-4 py-1.5 rounded-lg text-xs font-semibold transition ${
                  mode === 'text'
                    ? 'bg-white dark:bg-slate-800 text-emerald-600 dark:text-emerald-400 shadow-sm'
                    : 'text-slate-600 dark:text-slate-400 hover:text-slate-900'
                }`}
              >
                📝 Text Message
              </button>
              <button
                type="button"
                onClick={() => setMode('file')}
                class={`px-4 py-1.5 rounded-lg text-xs font-semibold transition ${
                  mode === 'file'
                    ? 'bg-white dark:bg-slate-800 text-emerald-600 dark:text-emerald-400 shadow-sm'
                    : 'text-slate-600 dark:text-slate-400 hover:text-slate-900'
                }`}
              >
                📎 Binary / File Attachment
              </button>
            </div>
          </div>

          {/* Payload Content Area */}
          {mode === 'text' ? (
            <div>
              <label class="block text-xs font-bold uppercase tracking-wider text-slate-600 dark:text-slate-300 mb-2">
                Message Content
              </label>
              <textarea
                rows={5}
                value={textPayload}
                onInput={(e) => setTextPayload((e.target as HTMLTextAreaElement).value)}
                placeholder="Type your message to transmit over DTN BPv7..."
                class="w-full px-4 py-3 rounded-xl border border-slate-300 dark:border-slate-600 bg-slate-50 dark:bg-slate-900 font-sans text-sm text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-emerald-500 transition"
              />
              <div class="mt-1 text-right text-xs text-slate-400 font-mono">
                {new Blob([textPayload]).size} bytes (UTF-8)
              </div>
            </div>
          ) : (
            <div>
              <label class="block text-xs font-bold uppercase tracking-wider text-slate-600 dark:text-slate-300 mb-2">
                Select File
              </label>
              <div class="border-2 border-dashed border-slate-300 dark:border-slate-600 rounded-2xl p-6 text-center hover:border-emerald-500 dark:hover:border-emerald-400 transition bg-slate-50/50 dark:bg-slate-900/40">
                <input
                  type="file"
                  id="file-upload"
                  class="hidden"
                  onChange={(e) => {
                    const files = (e.target as HTMLInputElement).files;
                    if (files && files.length > 0) {
                      setSelectedFile(files[0]);
                    }
                  }}
                />
                <label for="file-upload" class="cursor-pointer flex flex-col items-center">
                  <span class="text-3xl mb-2">📁</span>
                  <span class="text-sm font-semibold text-emerald-600 dark:text-emerald-400">
                    Click to browse file
                  </span>
                  <span class="text-xs text-slate-400 mt-1">
                    Any binary or text file (images, sensor data, bulletins)
                  </span>
                </label>
                {selectedFile && (
                  <div class="mt-4 p-2.5 bg-white dark:bg-slate-800 rounded-xl border border-slate-200 dark:border-slate-700 text-xs font-mono inline-flex items-center gap-2">
                    <span class="font-bold text-slate-800 dark:text-slate-200">{selectedFile.name}</span>
                    <span class="text-slate-400">({(selectedFile.size / 1024).toFixed(1)} KB)</span>
                    <button
                      type="button"
                      onClick={() => setSelectedFile(null)}
                      class="text-red-500 hover:text-red-700 ml-1 font-bold"
                    >
                      ✕
                    </button>
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Submit Button */}
          <div class="pt-2">
            <button
              type="submit"
              disabled={loading}
              class="w-full py-3 px-6 rounded-xl bg-gradient-to-r from-emerald-600 to-teal-500 hover:from-emerald-700 hover:to-teal-600 text-white font-bold text-sm shadow-md hover:shadow-lg transition flex items-center justify-center space-x-2 disabled:opacity-50"
            >
              {loading ? (
                <>
                  <div class="animate-spin rounded-full h-4 w-4 border-2 border-white border-t-transparent" />
                  <span>Dispatching to BPA...</span>
                </>
              ) : (
                <>
                  <span>🚀</span>
                  <span>Transmit Bundle</span>
                </>
              )}
            </button>
          </div>
        </form>

        {/* Feedback Area */}
        {error && (
          <div class="mx-6 mb-6 p-4 rounded-xl bg-red-50 dark:bg-red-950/60 border border-red-200 dark:border-red-800 text-red-800 dark:text-red-200 text-sm flex items-start gap-3">
            <span class="text-lg">❌</span>
            <div>
              <div class="font-bold">Transmission Failed</div>
              <div class="text-xs mt-1 text-red-700 dark:text-red-300">{error}</div>
            </div>
          </div>
        )}

        {result && (
          <div class="mx-6 mb-6 p-5 rounded-2xl bg-emerald-50 dark:bg-emerald-950/50 border border-emerald-200 dark:border-emerald-800 text-emerald-900 dark:text-emerald-100 space-y-3">
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-2 font-bold text-sm text-emerald-700 dark:text-emerald-300">
                <span class="text-lg">✅</span>
                <span>Bundle Successfully Dispatched</span>
              </div>
              <span class="text-xs px-2 py-0.5 rounded-full bg-emerald-100 dark:bg-emerald-900 font-semibold text-emerald-800 dark:text-emerald-200">
                {result.status.toUpperCase()}
              </span>
            </div>

            <div class="bg-white dark:bg-slate-900/90 p-3 rounded-xl border border-emerald-200 dark:border-emerald-800/80 space-y-2 text-xs font-mono">
              <div class="flex items-center justify-between">
                <span class="text-slate-400">Bundle ID:</span>
                <button
                  type="button"
                  onClick={copyBundleId}
                  class="px-2 py-0.5 rounded bg-slate-100 dark:bg-slate-800 hover:bg-slate-200 text-[11px] font-sans font-medium text-slate-700 dark:text-slate-300 transition"
                >
                  {copied ? 'Copied! ✓' : 'Copy Key'}
                </button>
              </div>
              <div class="text-emerald-600 dark:text-emerald-400 break-all select-all font-semibold">
                {result.bundle_id}
              </div>

              <div class="pt-2 border-t border-slate-100 dark:border-slate-800 grid grid-cols-2 gap-2 text-[11px] text-slate-500 dark:text-slate-400">
                <div>From: <span class="text-slate-800 dark:text-slate-200 font-semibold">{result.source}</span></div>
                <div>To: <span class="text-slate-800 dark:text-slate-200 font-semibold">{result.destination}</span></div>
                <div>Payload: <span class="text-slate-800 dark:text-slate-200">{result.payload_size} bytes</span></div>
                <div>Lifetime: <span class="text-slate-800 dark:text-slate-200">{result.lifetime_secs}s</span></div>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
