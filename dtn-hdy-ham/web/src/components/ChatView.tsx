import { h } from 'preact';
import { useState, useEffect, useRef } from 'preact/hooks';

export interface ChatMessage {
  id: string;
  conversation_peer: string;
  sender_eid: string;
  recipient_eid: string;
  content: string;
  direction: 'inbound' | 'outbound';
  status: 'queued' | 'sent' | 'received';
  is_read: boolean;
  timestamp: number;
  created_at: string;
}

export interface ConversationSummary {
  peer: string;
  callsign?: string;
  last_message: string;
  last_direction: 'inbound' | 'outbound';
  last_status: 'queued' | 'sent' | 'received';
  last_timestamp: number;
  last_created_at: string;
  unread_count: number;
  total_messages: number;
}

export function normalizeChatTarget(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) return 'dtn:none';
  if (trimmed.startsWith('ipn:')) return trimmed;

  if (trimmed.startsWith('dtn://')) {
    const stripped = trimmed.slice(6);
    const parts = stripped.split('/').filter((p) => p.trim().length > 0);
    if (parts.length === 0) {
      return 'dtn://none/chat';
    } else if (parts.len === 1 || parts.length === 1) {
      return `dtn://${parts[0].toLowerCase()}/chat`;
    } else {
      const node = parts[0].toLowerCase();
      const service = parts.slice(1).join('/');
      return `dtn://${node}/${service}`;
    }
  } else {
    const clean = trimmed.replace(/^\/+|\/+$/g, '').toLowerCase();
    return `dtn://${clean}/chat`;
  }
}

interface ChatViewProps {
  stationCallsign?: string;
  stationNodeId?: string;
  peerSuggestions?: string[];
  lastReceivedChatMessage?: ChatMessage | null;
  onUnreadTotalChange?: (total: number) => void;
}

// Format short timestamp: "14:32" for today, or "Nov 4, 14:32"
function formatMessageTime(isoStr: string | number): string {
  try {
    const d = typeof isoStr === 'number' ? new Date(isoStr) : new Date(isoStr);
    if (isNaN(d.getTime())) return '';
    const now = new Date();
    const isToday =
      d.getDate() === now.getDate() &&
      d.getMonth() === now.getMonth() &&
      d.getFullYear() === now.getFullYear();

    const timeStr = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    if (isToday) return timeStr;
    return `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })} ${timeStr}`;
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

// Render lightweight Markdown (bold, italic, code, links, newlines) safely
function renderMarkdown(content: string): string {
  const safe = escapeHtml(content);

  // Fenced code blocks ```code```
  let formatted = safe.replace(/```([\s\S]*?)```/g, (_match, p1) => {
    return `<pre class="my-1.5 p-2 bg-slate-900 text-slate-100 rounded text-xs font-mono overflow-x-auto whitespace-pre-wrap">${p1.trim()}</pre>`;
  });

  // Inline code `code`
  formatted = formatted.replace(/`([^`]+)`/g, '<code class="px-1 py-0.5 bg-slate-200 dark:bg-slate-700 rounded text-xs font-mono">$1</code>');

  // Bold **text**
  formatted = formatted.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');

  // Italic *text* or _text_
  formatted = formatted.replace(/\*([^*]+)\*/g, '<em>$1</em>');
  formatted = formatted.replace(/_([^_]+)_/g, '<em>$1</em>');

  // Links https:// or dtn://
  formatted = formatted.replace(
    /(https?:\/\/[^\s]+|dtn:\/\/[^\s]+)/g,
    '<a href="$1" target="_blank" rel="noopener noreferrer" class="underline hover:opacity-80">$1</a>'
  );

  // Line breaks
  formatted = formatted.replace(/\n/g, '<br/>');

  return formatted;
}

export function ChatView({
  stationCallsign,
  stationNodeId = 'dtn://station/',
  peerSuggestions = [],
  lastReceivedChatMessage,
  onUnreadTotalChange,
}: ChatViewProps) {
  const [conversations, setConversations] = useState<ConversationSummary[]>([]);
  const [selectedPeer, setSelectedPeer] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [inputText, setInputText] = useState('');
  const [loading, setLoading] = useState(false);
  const [sending, setSending] = useState(false);
  const [searchFilter, setSearchFilter] = useState('');
  const [showNewChatModal, setShowNewChatModal] = useState(false);
  const [newChatInput, setNewChatInput] = useState('');
  const [error, setError] = useState<string | null>(null);

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Fetch all conversation summaries
  const fetchConversations = async (keepSelection?: string) => {
    try {
      const res = await fetch('/api/v1/chat/conversations');
      if (res.ok) {
        const data: ConversationSummary[] = await res.json();
        setConversations(data);

        // Update total unread
        const totalUnread = data.reduce((acc, c) => acc + c.unread_count, 0);
        if (onUnreadTotalChange) {
          onUnreadTotalChange(totalUnread);
        }

        // Auto-select first conversation if none selected and on desktop
        if (!keepSelection && !selectedPeer && data.length > 0 && window.innerWidth >= 768) {
          selectConversation(data[0].peer);
        }
      }
    } catch (e: any) {
      console.error('Failed to load chat conversations', e);
    }
  };

  // Fetch message history for a peer
  const fetchMessages = async (peer: string) => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch(`/api/v1/chat/messages?peer=${encodeURIComponent(peer)}`);
      if (res.ok) {
        const data: ChatMessage[] = await res.json();
        setMessages(data);

        // Mark as read immediately on viewing
        await fetch('/api/v1/chat/read', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ peer }),
        });

        // Update local unread status for this conversation
        setConversations((prev) =>
          prev.map((c) => (c.peer === peer ? { ...c, unread_count: 0 } : c))
        );
      } else {
        const errJson = await res.json();
        throw new Error(errJson.error || `HTTP ${res.status}`);
      }
    } catch (e: any) {
      setError(e.message || 'Failed to load conversation history');
    } finally {
      setLoading(false);
    }
  };

  // Select a conversation thread
  const selectConversation = (peer: string) => {
    setSelectedPeer(peer);
    fetchMessages(peer);
  };

  // Initial load
  useEffect(() => {
    fetchConversations();
  }, []);

  // Handle incoming live SSE chat messages
  useEffect(() => {
    if (!lastReceivedChatMessage) return;
    const msg = lastReceivedChatMessage;

    // Refresh conversation list to get updated snippets & counts
    fetchConversations(selectedPeer || undefined);

    // If belongs to currently opened conversation, append dynamically
    if (selectedPeer && (msg.conversation_peer === selectedPeer || msg.sender_eid.includes(selectedPeer))) {
      setMessages((prev) => {
        // avoid duplicate if already present
        if (prev.some((m) => m.id === msg.id)) return prev;
        return [...prev, msg];
      });

      // Automatically mark as read if active
      if (msg.direction === 'inbound') {
        fetch('/api/v1/chat/read', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ peer: selectedPeer }),
        });
      }
    }
  }, [lastReceivedChatMessage]);

  // Scroll to bottom when messages change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  // Send message
  const handleSendMessage = async (e?: Event) => {
    if (e) e.preventDefault();
    const content = inputText.trim();
    if (!content || !selectedPeer || sending) return;

    setSending(true);
    setError(null);

    try {
      const res = await fetch('/api/v1/chat/messages', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          peer: selectedPeer,
          content,
        }),
      });

      if (!res.ok) {
        const errData = await res.json();
        throw new Error(errData.error || `Failed to send: HTTP ${res.status}`);
      }

      const sentMsg: ChatMessage = await res.json();
      setMessages((prev) => [...prev, sentMsg]);
      setInputText('');

      // Refresh list to show latest snippet
      fetchConversations(selectedPeer);

      // Focus back to input
      inputRef.current?.focus();
    } catch (err: any) {
      setError(err.message || 'Failed to dispatch chat message');
    } finally {
      setSending(false);
    }
  };

  // Start new conversation from modal
  const handleStartNewChat = async () => {
    const raw = newChatInput.trim();
    if (!raw) return;

    const target = normalizeChatTarget(raw);
    setShowNewChatModal(false);
    setNewChatInput('');

    try {
      await fetch('/api/v1/chat/conversations', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ peer: target }),
      });
      await fetchConversations(target);
    } catch (e) {
      console.error('Failed to create persistent conversation', e);
    }

    selectConversation(target);
  };

  // Filter conversations
  const filteredConversations = conversations.filter((c) => {
    const q = searchFilter.toLowerCase();
    return (
      c.peer.toLowerCase().includes(q) ||
      (c.last_message && c.last_message.toLowerCase().includes(q))
    );
  });

  const selectedConv = conversations.find((c) => c.peer === selectedPeer);
  const activeTitle = selectedPeer || '';

  return (
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
      {/* Top Banner / Breadcrumb */}
      <div class="mb-4 flex items-center justify-between">
        <div>
          <h1 class="text-2xl font-bold tracking-tight text-slate-900 dark:text-white flex items-center space-x-2">
            <span>💬</span>
            <span>Interactive Chat</span>
          </h1>
          <p class="text-sm text-slate-500 dark:text-slate-400">
            Amateur radio store-and-forward messaging via endpoint{' '}
            <code class="px-1.5 py-0.5 rounded bg-slate-200 dark:bg-slate-700 font-mono text-xs text-emerald-700 dark:text-emerald-300">
              {stationNodeId.replace(/\/$/, '')}/chat
            </code>
          </p>
        </div>

        <button
          onClick={() => setShowNewChatModal(true)}
          class="inline-flex items-center space-x-1.5 px-3.5 py-2 rounded-lg text-sm font-medium bg-emerald-600 hover:bg-emerald-500 text-white shadow-sm transition-colors"
        >
          <span>✏️</span>
          <span>+ New Chat</span>
        </button>
      </div>

      {/* Main Dual-Pane Window */}
      <div class="bg-white dark:bg-slate-800 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-700 overflow-hidden flex h-[720px] max-h-[82vh]">
        {/* Left Sidebar: Conversations List */}
        <aside
          class={`w-full md:w-80 lg:w-96 flex-shrink-0 border-r border-slate-200 dark:border-slate-700 flex flex-col bg-slate-50/50 dark:bg-slate-800/50 ${
            selectedPeer ? 'hidden md:flex' : 'flex'
          }`}
        >
          {/* Search Header */}
          <div class="p-3.5 border-b border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800">
            <div class="relative">
              <span class="absolute inset-y-0 left-0 pl-3 flex items-center text-slate-400 text-sm">
                🔍
              </span>
              <input
                type="text"
                placeholder="Search callsign or EID..."
                value={searchFilter}
                onInput={(e) => setSearchFilter((e.target as HTMLInputElement).value)}
                class="w-full pl-9 pr-3 py-1.5 text-sm bg-slate-100 dark:bg-slate-700/60 text-slate-900 dark:text-white rounded-lg border border-transparent focus:border-emerald-500 focus:bg-white dark:focus:bg-slate-800 outline-none transition"
              />
            </div>
          </div>

          {/* Conversations Thread List */}
          <div class="flex-1 overflow-y-auto divide-y divide-slate-100 dark:divide-slate-700/50">
            {filteredConversations.length === 0 ? (
              <div class="p-8 text-center text-slate-400">
                <span class="text-3xl block mb-2">📭</span>
                <p class="text-sm font-medium text-slate-600 dark:text-slate-300">
                  No conversations yet
                </p>
                <p class="text-xs text-slate-400 mt-1">
                  Click "+ New Chat" above to reach a station or wait for incoming bundles.
                </p>
              </div>
            ) : (
              filteredConversations.map((c) => {
                const isSelected = c.peer === selectedPeer;
                const title = c.peer;

                return (
                  <button
                    key={c.peer}
                    onClick={() => selectConversation(c.peer)}
                    class={`w-full text-left p-3.5 flex items-start space-x-3 transition-colors ${
                      isSelected
                        ? 'bg-emerald-50 dark:bg-emerald-950/40 border-l-4 border-emerald-600 dark:border-emerald-500'
                        : 'hover:bg-slate-100/70 dark:hover:bg-slate-750'
                    }`}
                  >
                    {/* Station Avatar Icon */}
                    <div class="w-10 h-10 rounded-full flex-shrink-0 flex items-center justify-center font-bold text-xs bg-gradient-to-tr from-emerald-600 to-teal-500 text-white shadow-sm font-mono">
                      {c.peer.replace(/^dtn:\/\//, '').substring(0, 2).toUpperCase()}
                    </div>

                    <div class="flex-1 min-w-0">
                      <div class="flex items-center justify-between mb-0.5">
                        <span class="font-bold text-sm text-slate-900 dark:text-white truncate font-mono">
                          {title}
                        </span>
                        <span class="text-xs text-slate-400 whitespace-nowrap ml-2">
                          {formatMessageTime(c.last_created_at)}
                        </span>
                      </div>

                      <p class="text-xs text-slate-500 dark:text-slate-400 truncate flex items-center space-x-1">
                        <span class="opacity-70">
                          {c.last_direction === 'outbound' ? '📤' : '📥'}
                        </span>
                        <span class="truncate">{c.last_message || '(No message)'}</span>
                      </p>
                    </div>

                    {/* Unread badge */}
                    {c.unread_count > 0 && (
                      <span class="ml-2 inline-flex items-center justify-center px-2 py-0.5 text-xs font-bold leading-none text-white bg-emerald-600 rounded-full shadow-sm">
                        {c.unread_count}
                      </span>
                    )}
                  </button>
                );
              })
            )}
          </div>
        </aside>

        {/* Right Pane: Active Thread */}
        <section
          class={`flex-1 flex flex-col bg-white dark:bg-slate-800 ${
            !selectedPeer ? 'hidden md:flex' : 'flex'
          }`}
        >
          {selectedPeer ? (
            <>
              {/* Active Conversation Header */}
              <div class="px-4 py-3.5 border-b border-slate-200 dark:border-slate-700 flex items-center justify-between bg-slate-50/70 dark:bg-slate-800/80">
                <div class="flex items-center space-x-3">
                  {/* Back button for mobile */}
                  <button
                    onClick={() => setSelectedPeer(null)}
                    class="md:hidden p-1.5 text-slate-500 hover:text-slate-800 dark:hover:text-white rounded-lg"
                  >
                    ←
                  </button>

                  <div class="w-9 h-9 rounded-full flex items-center justify-center font-bold text-xs bg-gradient-to-tr from-emerald-600 to-teal-500 text-white font-mono">
                    {selectedPeer.replace(/^dtn:\/\//, '').substring(0, 2).toUpperCase()}
                  </div>

                  <div>
                    <h2 class="font-bold text-slate-900 dark:text-white flex items-center space-x-2">
                      <span class="font-mono">{selectedPeer}</span>
                    </h2>
                    <p class="text-xs font-mono text-slate-400">
                      Destination EID: {selectedPeer}
                    </p>
                  </div>
                </div>

                <div class="flex items-center space-x-2">
                  <button
                    onClick={() => fetchMessages(selectedPeer)}
                    title="Reload thread"
                    class="p-2 text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 rounded-lg transition"
                  >
                    🔄
                  </button>
                </div>
              </div>

              {/* Error Alert */}
              {error && (
                <div class="mx-4 my-2 p-2.5 rounded-lg bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800 text-xs text-red-600 dark:text-red-400">
                  {error}
                </div>
              )}

              {/* Messages Bubble Thread */}
              <div class="flex-1 p-4 overflow-y-auto space-y-3 bg-slate-100/40 dark:bg-slate-900/40">
                {loading && messages.length === 0 ? (
                  <div class="flex justify-center items-center h-full text-slate-400 text-sm">
                    <span>Loading conversation...</span>
                  </div>
                ) : messages.length === 0 ? (
                  <div class="flex flex-col items-center justify-center h-full text-slate-400 text-center">
                    <span class="text-4xl mb-2">💬</span>
                    <p class="font-medium text-slate-600 dark:text-slate-300">No messages yet</p>
                    <p class="text-xs text-slate-400 mt-1 max-w-sm">
                      Type your message below and press Enter to transmit to{' '}
                      <span class="font-mono text-emerald-600 dark:text-emerald-400">{activeTitle}</span>.
                    </p>
                  </div>
                ) : (
                  messages.map((m) => {
                    const isOutbound = m.direction === 'outbound';

                    return (
                      <div
                        key={m.id}
                        class={`flex flex-col ${isOutbound ? 'items-end' : 'items-start'}`}
                      >
                        <div
                          class={`max-w-[80%] sm:max-w-[70%] rounded-2xl px-4 py-2.5 shadow-sm text-sm ${
                            isOutbound
                              ? 'bg-emerald-600 text-white dark:bg-emerald-700 rounded-br-none'
                              : 'bg-white dark:bg-slate-800 text-slate-900 dark:text-slate-100 border border-slate-200 dark:border-slate-700 rounded-bl-none'
                          }`}
                        >
                          {/* Markdown message body */}
                          <div
                            class="prose prose-sm dark:prose-invert break-words"
                            dangerouslySetInnerHTML={{ __html: renderMarkdown(m.content) }}
                          />

                          {/* Footer: timestamp & delivery status */}
                          <div
                            class={`mt-1 flex items-center justify-end space-x-1 text-[10px] ${
                              isOutbound ? 'text-emerald-100' : 'text-slate-400'
                            }`}
                          >
                            <span>{formatMessageTime(m.created_at)}</span>
                            {isOutbound && (
                              <span title={`Status: ${m.status}`}>
                                {m.status === 'queued' ? '⏳' : '📤'}
                              </span>
                            )}
                          </div>
                        </div>
                      </div>
                    );
                  })
                )}
                <div ref={messagesEndRef} />
              </div>

              {/* Message Input Box */}
              <div class="p-3.5 border-t border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800">
                <form onSubmit={handleSendMessage} class="flex items-end space-x-2">
                  <div class="flex-1 relative">
                    <textarea
                      ref={inputRef}
                      rows={2}
                      value={inputText}
                      placeholder={`Message ${activeTitle}... (Markdown & emojis supported)`}
                      onInput={(e) => setInputText((e.target as HTMLTextAreaElement).value)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter' && !e.shiftKey) {
                          e.preventDefault();
                          handleSendMessage();
                        }
                      }}
                      class="w-full resize-none p-2.5 text-sm bg-slate-50 dark:bg-slate-900 text-slate-900 dark:text-white rounded-xl border border-slate-200 dark:border-slate-700 focus:border-emerald-500 focus:ring-1 focus:ring-emerald-500 outline-none transition"
                    />
                    <div class="text-[10px] text-slate-400 absolute right-2.5 bottom-2 hidden sm:block">
                      Press <kbd class="px-1 py-0.5 rounded bg-slate-200 dark:bg-slate-700 text-slate-600 dark:text-slate-300">Enter</kbd> to send
                    </div>
                  </div>

                  <button
                    type="submit"
                    disabled={sending || !inputText.trim()}
                    class="h-[48px] px-4 rounded-xl font-medium text-sm bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white flex items-center justify-center space-x-1.5 shadow-sm transition"
                  >
                    <span>{sending ? '⏳' : '📤'}</span>
                    <span class="hidden sm:inline">Send</span>
                  </button>
                </form>
              </div>
            </>
          ) : (
            /* Empty State when no conversation selected */
            <div class="flex-1 flex flex-col items-center justify-center p-8 text-center bg-slate-50/50 dark:bg-slate-850">
              <div class="w-16 h-16 rounded-2xl bg-emerald-100 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 flex items-center justify-center text-3xl shadow-sm mb-4">
                💬
              </div>
              <h3 class="text-lg font-bold text-slate-800 dark:text-white">
                Hardy Chat Messenger
              </h3>
              <p class="text-sm text-slate-500 dark:text-slate-400 mt-1 max-w-md">
                Select a conversation thread from the left or start a new direct chat with an amateur radio correspondent.
              </p>
              <button
                onClick={() => setShowNewChatModal(true)}
                class="mt-4 px-4 py-2 rounded-xl text-sm font-semibold bg-emerald-600 hover:bg-emerald-500 text-white shadow-sm transition"
              >
                + Start New Chat
              </button>
            </div>
          )}
        </section>
      </div>

      {/* New Chat Modal */}
      {showNewChatModal && (
        <div class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-sm">
          <div class="bg-white dark:bg-slate-800 rounded-2xl shadow-xl max-w-md w-full border border-slate-200 dark:border-slate-700 overflow-hidden">
            <div class="px-5 py-4 border-b border-slate-100 dark:border-slate-700 flex items-center justify-between">
              <h3 class="font-bold text-slate-900 dark:text-white flex items-center space-x-2">
                <span>✏️</span>
                <span>Start New Chat</span>
              </h3>
              <button
                onClick={() => setShowNewChatModal(false)}
                class="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 text-lg leading-none"
              >
                ✕
              </button>
            </div>

            <div class="p-5 space-y-4">
              <div>
                <label class="block text-xs font-semibold text-slate-700 dark:text-slate-300 uppercase tracking-wider mb-1.5">
                  Correspondent Callsign or EID
                </label>
                <input
                  type="text"
                  placeholder="e.g. F4JXQ or dtn://f4jxq/"
                  value={newChatInput}
                  onInput={(e) => setNewChatInput((e.target as HTMLInputElement).value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      handleStartNewChat();
                    }
                  }}
                  autoFocus
                  class="w-full px-3.5 py-2 text-sm bg-slate-50 dark:bg-slate-900 text-slate-900 dark:text-white rounded-xl border border-slate-300 dark:border-slate-600 focus:border-emerald-500 outline-none"
                />
                <p class="text-xs text-slate-400 mt-1">
                  Bundles will be dispatched to endpoint{' '}
                  <span class="font-mono text-emerald-600 dark:text-emerald-400">
                    {newChatInput.trim() ? normalizeChatTarget(newChatInput) : 'dtn://<callsign>/chat'}
                  </span>
                </p>
              </div>

              {/* Suggestions chips */}
              {peerSuggestions.length > 0 && (
                <div>
                  <label class="block text-xs font-medium text-slate-500 mb-1.5">
                    Known Station Peers:
                  </label>
                  <div class="flex flex-wrap gap-1.5">
                    {peerSuggestions.map((p) => (
                      <button
                        key={p}
                        type="button"
                        onClick={() => setNewChatInput(p)}
                        class="px-2.5 py-1 text-xs rounded-lg bg-slate-100 hover:bg-slate-200 dark:bg-slate-700 dark:hover:bg-slate-600 text-slate-700 dark:text-slate-200 font-mono transition"
                      >
                        {p}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>

            <div class="px-5 py-3.5 bg-slate-50 dark:bg-slate-750 flex justify-end space-x-2">
              <button
                type="button"
                onClick={() => setShowNewChatModal(false)}
                class="px-4 py-2 text-sm text-slate-600 dark:text-slate-300 hover:bg-slate-200/60 dark:hover:bg-slate-700 rounded-xl transition"
              >
                Cancel
              </button>
              <button
                type="button"
                disabled={!newChatInput.trim()}
                onClick={handleStartNewChat}
                class="px-4 py-2 text-sm font-semibold bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white rounded-xl shadow-sm transition"
              >
                Open Chat
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
