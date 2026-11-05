import { h } from 'preact';

interface NavbarProps {
  activeTab: 'config' | 'chat' | 'files' | 'bundles' | 'send';
  onTabChange: (tab: 'config' | 'chat' | 'files' | 'bundles' | 'send') => void;
  darkMode: boolean;
  onToggleDarkMode: () => void;
  stationCallsign?: string;
  nodeId?: string;
  unreadChatCount?: number;
  unreadFilesCount?: number;
}

export function Navbar({
  activeTab,
  onTabChange,
  darkMode,
  onToggleDarkMode,
  stationCallsign,
  nodeId,
  unreadChatCount = 0,
  unreadFilesCount = 0,
}: NavbarProps) {
  return (
    <header class="bg-white dark:bg-slate-800 border-b border-slate-200 dark:border-slate-700 shadow-sm sticky top-0 z-30">
      <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 flex items-center justify-between h-16">
        <div class="flex items-center space-x-3">
          <div class="flex items-center justify-center w-10 h-10 rounded-xl bg-gradient-to-tr from-emerald-600 to-teal-400 text-white shadow-md">
            <span class="text-xl">📡</span>
          </div>
          <div>
            <div class="flex items-center space-x-2">
              <span class="text-lg font-bold tracking-tight text-slate-900 dark:text-white">
                Hardy BPA
              </span>
              <span class="px-2 py-0.5 text-xs font-semibold rounded-full bg-emerald-100 text-emerald-800 dark:bg-emerald-900/60 dark:text-emerald-300">
                Hamradio
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

        {/* Navigation Tabs */}
        <nav aria-label="Main navigation" class="flex items-center space-x-1 sm:space-x-2">
          <button
            onClick={() => onTabChange('config')}
            aria-current={activeTab === 'config' ? 'page' : undefined}
            class={`px-3 sm:px-4 py-2 text-sm font-medium rounded-lg transition-colors flex items-center space-x-2 ${
              activeTab === 'config'
                ? 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800'
                : 'text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/50'
            }`}
          >
            <span aria-hidden="true">⚙️</span>
            <span>Configuration</span>
          </button>

          <button
            onClick={() => onTabChange('chat')}
            aria-current={activeTab === 'chat' ? 'page' : undefined}
            class={`px-3 sm:px-4 py-2 text-sm font-medium rounded-lg transition-colors flex items-center space-x-2 relative ${
              activeTab === 'chat'
                ? 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800'
                : 'text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/50'
            }`}
          >
            <span aria-hidden="true">💬</span>
            <span>Chat</span>
            {unreadChatCount > 0 && (
              <span class="inline-flex items-center justify-center px-1.5 py-0.5 text-xs font-bold leading-none text-white bg-emerald-600 rounded-full shadow-sm ml-1" aria-label={`${unreadChatCount} unread messages`}>
                {unreadChatCount}
              </span>
            )}
          </button>

          <button
            onClick={() => onTabChange('files')}
            aria-current={activeTab === 'files' ? 'page' : undefined}
            class={`px-3 sm:px-4 py-2 text-sm font-medium rounded-lg transition-colors flex items-center space-x-2 relative ${
              activeTab === 'files'
                ? 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800'
                : 'text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/50'
            }`}
          >
            <span aria-hidden="true">📁</span>
            <span>Files</span>
            {unreadFilesCount > 0 && (
              <span class="inline-flex items-center justify-center px-1.5 py-0.5 text-xs font-bold leading-none text-white bg-emerald-600 rounded-full shadow-sm ml-1" aria-label={`${unreadFilesCount} unread files`}>
                {unreadFilesCount}
              </span>
            )}
          </button>

          <button
            onClick={() => onTabChange('bundles')}
            aria-current={activeTab === 'bundles' ? 'page' : undefined}
            class={`px-3 sm:px-4 py-2 text-sm font-medium rounded-lg transition-colors flex items-center space-x-2 ${
              activeTab === 'bundles'
                ? 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800'
                : 'text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/50'
            }`}
          >
            <span aria-hidden="true">📦</span>
            <span>Bundles</span>
          </button>

          <button
            onClick={() => onTabChange('send')}
            aria-current={activeTab === 'send' ? 'page' : undefined}
            class={`px-3 sm:px-4 py-2 text-sm font-medium rounded-lg transition-colors flex items-center space-x-2 ${
              activeTab === 'send'
                ? 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800'
                : 'text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/50'
            }`}
          >
            <span aria-hidden="true">✉️</span>
            <span>Send Bundle</span>
          </button>

          <div class="h-6 w-px bg-slate-200 dark:bg-slate-700 mx-1" />

          {/* Theme toggle */}
          <button
            onClick={onToggleDarkMode}
            title={darkMode ? 'Switch to light mode' : 'Switch to dark mode'}
            aria-label={darkMode ? 'Switch to light mode' : 'Switch to dark mode'}
            aria-pressed={darkMode}
            class="p-2 text-slate-500 dark:text-slate-400 hover:text-slate-800 dark:hover:text-slate-200 rounded-lg hover:bg-slate-100 dark:hover:bg-slate-700/50 transition-colors"
          >
            <span aria-hidden="true">{darkMode ? '☀️' : '🌙'}</span>
          </button>
        </nav>
      </div>
    </header>
  );
}
