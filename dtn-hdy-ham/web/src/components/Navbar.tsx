import { h } from 'preact';

interface NavbarProps {
  activeTab: 'config' | 'chat' | 'files' | 'bundles' | 'send' | 'logs';
  onTabChange: (tab: 'config' | 'chat' | 'files' | 'bundles' | 'send' | 'logs') => void;
  darkMode: boolean;
  onToggleDarkMode: () => void;
  soundEnabled: boolean;
  onToggleSound: () => void;
  sseStatus: 'connected' | 'disconnected' | 'connecting';
  stationCallsign?: string;
  nodeId?: string;
  unreadChatCount?: number;
  unreadFilesCount?: number;
  canInstall?: boolean;
  onInstall?: () => void;
}

export function Navbar({
  activeTab,
  onTabChange,
  darkMode,
  onToggleDarkMode,
  soundEnabled,
  onToggleSound,
  sseStatus,
  stationCallsign,
  nodeId,
  unreadChatCount = 0,
  unreadFilesCount = 0,
  canInstall = false,
  onInstall,
}: NavbarProps) {
  const tabs = [
    { id: 'config' as const, label: 'Configuration', shortLabel: 'Config', icon: '⚙️' },
    { id: 'chat' as const, label: 'Chat', shortLabel: 'Chat', icon: '💬', count: unreadChatCount },
    { id: 'files' as const, label: 'Files', shortLabel: 'Files', icon: '📁', count: unreadFilesCount },
    { id: 'bundles' as const, label: 'Bundles', shortLabel: 'Bundles', icon: '📦' },
    { id: 'send' as const, label: 'Send Bundle', shortLabel: 'Send', icon: '✉️' },
    { id: 'logs' as const, label: 'Logs', shortLabel: 'Logs', icon: '📟' },
  ];

  return (
    <>
      {/* Top Header */}
      <header class="bg-white dark:bg-slate-800 border-b border-slate-200 dark:border-slate-700 shadow-sm sticky top-0 z-30">
        <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 flex items-center justify-between h-16">
          <div class="flex items-center space-x-3">
            <div class="flex items-center justify-center w-10 h-10 rounded-xl bg-gradient-to-tr from-emerald-600 to-teal-400 text-white shadow-md shrink-0">
              <span class="text-xl">📡</span>
            </div>
            <div>
              <div class="flex items-center space-x-2">
                <span class="text-base sm:text-lg font-bold tracking-tight text-slate-900 dark:text-white">
                  Hardy BPA
                </span>
                <span class="px-2 py-0.5 text-xs font-semibold rounded-full bg-emerald-100 text-emerald-800 dark:bg-emerald-900/60 dark:text-emerald-300">
                  Hamradio
                </span>
                {/* SSE Live status pill */}
                <span
                  class={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium border ${
                    sseStatus === 'connected'
                      ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300 border-emerald-300 dark:border-emerald-800'
                      : sseStatus === 'connecting'
                      ? 'bg-amber-50 text-amber-700 dark:bg-amber-950/60 dark:text-amber-300 border-amber-300 dark:border-amber-800'
                      : 'bg-rose-50 text-rose-700 dark:bg-rose-950/60 dark:text-rose-300 border-rose-300 dark:border-rose-800'
                  }`}
                  title={`SSE real-time stream is ${sseStatus}`}
                >
                  <span
                    class={`w-1.5 h-1.5 rounded-full ${
                      sseStatus === 'connected'
                        ? 'bg-emerald-500'
                        : sseStatus === 'connecting'
                        ? 'bg-amber-500'
                        : 'bg-rose-500'
                    }`}
                  />
                  <span class="hidden sm:inline">{sseStatus === 'connected' ? 'Live' : sseStatus}</span>
                </span>
              </div>
              <div class="text-xs text-slate-500 dark:text-slate-400 font-mono">
                {stationCallsign ? (
                  <span>
                    <strong class="text-emerald-600 dark:text-emerald-400 font-semibold">{stationCallsign}</strong>
                    {nodeId ? ` · ${nodeId}` : ''}
                  </span>
                ) : (
                  'DTN BPv7 Station'
                )}
              </div>
            </div>
          </div>

          {/* Desktop Navigation Tabs */}
          <nav aria-label="Main navigation" class="hidden md:flex items-center space-x-1 lg:space-x-2">
            {tabs.map((tab) => {
              const isActive = activeTab === tab.id;
              return (
                <button
                  key={tab.id}
                  onClick={() => onTabChange(tab.id)}
                  aria-current={isActive ? 'page' : undefined}
                  class={`px-3 py-2 text-sm font-medium rounded-lg transition-colors flex items-center space-x-1.5 relative ${
                    isActive
                      ? 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800'
                      : 'text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/50'
                  }`}
                >
                  <span aria-hidden="true">{tab.icon}</span>
                  <span>{tab.label}</span>
                  {tab.count !== undefined && tab.count > 0 && (
                    <span
                      class="inline-flex items-center justify-center px-1.5 py-0.5 text-xs font-bold leading-none text-white bg-emerald-600 rounded-full shadow-sm ml-1"
                      aria-label={`${tab.count} unread`}
                    >
                      {tab.count}
                    </span>
                  )}
                </button>
              );
            })}
          </nav>

          {/* Controls: Install, Sound & Theme */}
          <div class="flex items-center space-x-1.5">
            {canInstall && (
              <button
                onClick={onInstall}
                class="px-2.5 py-1.5 text-xs font-semibold rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white shadow-xs flex items-center space-x-1 transition-colors"
                title="Installer l'application sur cet appareil"
                aria-label="Installer l'application PWA sur cet appareil"
              >
                <span>📲</span>
                <span class="hidden sm:inline">Installer</span>
              </button>
            )}

            {/* Sound Notification toggle */}
            <button
              onClick={onToggleSound}
              title={soundEnabled ? 'Disable audio notification chime' : 'Enable audio notification chime on incoming messages'}
              aria-label={soundEnabled ? 'Disable audio notification chime' : 'Enable audio notification chime on incoming messages'}
              class={`p-2 rounded-lg transition-colors min-h-[44px] min-w-[44px] flex items-center justify-center ${
                soundEnabled
                  ? 'text-emerald-600 dark:text-emerald-400 hover:bg-emerald-50 dark:hover:bg-emerald-950/50'
                  : 'text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/50'
              }`}
            >
              <span aria-hidden="true" class="text-base">{soundEnabled ? '🔔' : '🔕'}</span>
            </button>

            {/* Theme toggle */}
            <button
              onClick={onToggleDarkMode}
              title={darkMode ? 'Switch to light mode' : 'Switch to dark mode'}
              aria-label={darkMode ? 'Switch to light mode' : 'Switch to dark mode'}
              aria-pressed={darkMode}
              class="p-2 text-slate-500 dark:text-slate-400 hover:text-slate-800 dark:hover:text-slate-200 rounded-lg hover:bg-slate-100 dark:hover:bg-slate-700/50 transition-colors min-h-[44px] min-w-[44px] flex items-center justify-center"
            >
              <span aria-hidden="true" class="text-base">{darkMode ? '☀️' : '🌙'}</span>
            </button>
          </div>
        </div>
      </header>

      {/* Mobile Bottom Navigation Bar (Fixed thumb zone) */}
      <nav
        aria-label="Mobile navigation"
        class="md:hidden fixed bottom-0 left-0 right-0 z-40 bg-white/95 dark:bg-slate-900/95 backdrop-blur-md border-t border-slate-200 dark:border-slate-800 pb-[env(safe-area-inset-bottom)] shadow-lg flex items-center justify-around h-16"
      >
        {tabs.map((tab) => {
          const isActive = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              onClick={() => onTabChange(tab.id)}
              aria-current={isActive ? 'page' : undefined}
              class={`flex-1 flex flex-col items-center justify-center py-1 min-h-[44px] min-w-[44px] text-[11px] font-medium transition-colors relative ${
                isActive
                  ? 'text-emerald-600 dark:text-emerald-400 font-bold'
                  : 'text-slate-500 dark:text-slate-400 hover:text-slate-800 dark:hover:text-slate-200'
              }`}
            >
              <span class="text-base leading-none mb-1" aria-hidden="true">{tab.icon}</span>
              <span class="truncate max-w-[54px]">{tab.shortLabel}</span>
              {tab.count !== undefined && tab.count > 0 && (
                <span
                  class="absolute top-1 right-2 inline-flex items-center justify-center px-1.5 py-0.5 text-[10px] font-bold leading-none text-white bg-emerald-600 rounded-full shadow-xs"
                  aria-label={`${tab.count} non-lus`}
                >
                  {tab.count}
                </span>
              )}
            </button>
          );
        })}
      </nav>
    </>
  );
}
