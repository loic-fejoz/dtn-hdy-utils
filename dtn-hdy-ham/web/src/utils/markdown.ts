// Common sanitized markdown and HTML utility for dtn-hdy-ham web interface

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;');
}

export function escapeAttribute(attr: string): string {
  return attr
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

// Validates whether an image URL is safe (data-uri of allowed image types or http/https)
export function isSafeImageUrl(url: string): boolean {
  const trimmed = url.trim();
  // Safe Data URIs for images (png, jpeg, gif, webp, avif, svg+xml)
  const isDataUri = /^data:image\/(?:png|jpeg|jpg|gif|webp|avif|svg\+xml);base64,[A-Za-z0-9+/=\s]+$/i.test(trimmed);
  if (isDataUri) {
    return true;
  }
  // Safe HTTP/HTTPS urls
  const isHttpUri = /^https?:\/\/[a-zA-Z0-9\-._~:/?#[\]@!$&'()*+,;=]+$/i.test(trimmed);
  return isHttpUri;
}

// Validates whether a link URL is safe (http, https, dtn, mailto, tel, relative paths)
export function isSafeLinkUrl(url: string): boolean {
  const trimmed = url.trim();
  if (!trimmed) return false;

  // Disallow control characters or newlines
  if (/[\x00-\x1f\x7f]/.test(trimmed)) {
    return false;
  }

  // Reject URLs containing HTML entity escapes (e.g. &#, &colon;) to prevent scheme obfuscation
  if (/&[#a-zA-Z0-9]+;/.test(trimmed)) {
    return false;
  }

  // Check if it has a URI scheme before any '/', '?', or '#'
  // RFC 3986: scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
  const schemeMatch = trimmed.match(/^([a-zA-Z][a-zA-Z0-9+.-]*):/);
  if (schemeMatch) {
    const scheme = schemeMatch[1].toLowerCase();
    // Only allow safe schemes
    return ['http', 'https', 'dtn', 'mailto', 'tel'].includes(scheme);
  }

  // Protocol-relative URLs: allow if followed by valid characters
  if (trimmed.startsWith('//')) {
    return /^\/\/[a-zA-Z0-9\-._~:/?#[\]@!$&'()*+,;=]+$/i.test(trimmed);
  }

  // Relative URLs: path, query, hash (e.g. "src/bin/dtnprint.rs", "./foo", "../bar", "/path", "#anchor")
  return /^[a-zA-Z0-9\-._~:/?#[\]@!$&'()*+,;=]+$/i.test(trimmed);
}

export function renderMarkdown(content: string, options: { allowImages?: boolean } = {}): string {
  const placeholders: { token: string; html: string }[] = [];
  let tokenCounter = 0;

  function createPlaceholder(html: string): string {
    const token = `@@HDYTOKEN${tokenCounter++}@@`;
    placeholders.push({ token, html });
    return token;
  }

  let textToProcess = content;

  // 1. Fenced code blocks ```code```
  textToProcess = textToProcess.replace(/```([a-zA-Z0-9_-]*)\n?([\s\S]*?)```/g, (_match, _lang, code) => {
    return createPlaceholder(`<pre class="my-2 p-3 bg-slate-900 text-slate-100 rounded-xl text-xs font-mono overflow-x-auto whitespace-pre-wrap">${escapeHtml(code.trim())}</pre>`);
  });

  // 2. Inline code `code`
  textToProcess = textToProcess.replace(/`([^`]+)`/g, (_match, code) => {
    return createPlaceholder(`<code class="px-1.5 py-0.5 bg-slate-200 dark:bg-slate-700 rounded text-xs font-mono">${escapeHtml(code)}</code>`);
  });

  // 3. Images ![alt](url)
  if (options.allowImages) {
    textToProcess = textToProcess.replace(/!\[(.*?)\]\((.*?)\)/g, (_match, alt, url) => {
      const cleanUrl = url.trim();
      if (!isSafeImageUrl(cleanUrl)) {
        return `[Image blocked: unsafe URL]`;
      }
      const safeAlt = escapeHtml(alt || 'Image');
      const safeUrl = escapeAttribute(cleanUrl);
      const tag = `<div class="my-3"><img src="${safeUrl}" alt="${safeAlt}" class="max-h-96 rounded-xl shadow border border-slate-200 dark:border-slate-700 object-contain mx-auto" /><p class="text-center text-xs text-slate-500 dark:text-slate-400 mt-1">${safeAlt}</p></div>`;
      return createPlaceholder(tag);
    });
  }

  // 4. Markdown links: [label](url "optional title")
  const linkRegex = /\[([^\]]+)\]\(\s*([^\s()]+(?:\([^\s()]+\)[^\s()]*)*)(?:\s+["']([^"']*)["'])?\s*\)/g;
  textToProcess = textToProcess.replace(linkRegex, (_match, label, url, title) => {
    const cleanUrl = url.trim();
    if (!isSafeLinkUrl(cleanUrl)) {
      return `[${escapeHtml(label)}](unsafe link blocked)`;
    }
    const safeUrl = escapeAttribute(cleanUrl);
    const titleAttr = title ? ` title="${escapeAttribute(title)}"` : '';
    let formattedLabel = escapeHtml(label);
    formattedLabel = formattedLabel.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    formattedLabel = formattedLabel.replace(/\*([^*]+)\*/g, '<em>$1</em>');
    formattedLabel = formattedLabel.replace(/_([^_]+)_/g, '<em>$1</em>');
    const tag = `<a href="${safeUrl}" target="_blank" rel="noopener noreferrer"${titleAttr} class="text-emerald-600 dark:text-emerald-400 underline hover:opacity-85">${formattedLabel}</a>`;
    return createPlaceholder(tag);
  });

  // 5. Escape HTML on remaining text
  let formatted = escapeHtml(textToProcess);

  // 6. Headings
  formatted = formatted.replace(/^### (.*$)/gm, '<h3 class="text-base font-bold text-slate-900 dark:text-white mt-3 mb-1">$1</h3>');
  formatted = formatted.replace(/^## (.*$)/gm, '<h2 class="text-lg font-bold text-emerald-600 dark:text-emerald-400 mt-4 mb-2">$1</h2>');
  formatted = formatted.replace(/^# (.*$)/gm, '<h1 class="text-xl font-extrabold text-slate-900 dark:text-white mt-4 mb-2 pb-1 border-b border-slate-200 dark:border-slate-700">$1</h1>');

  // 7. Bold & Italic
  formatted = formatted.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  formatted = formatted.replace(/\*([^*]+)\*/g, '<em>$1</em>');
  formatted = formatted.replace(/_([^_]+)_/g, '<em>$1</em>');

  // 8. Raw autolinks: match safe URLs ending at whitespace or quote/tag boundary
  formatted = formatted.replace(
    /(https?:\/\/[a-zA-Z0-9\-._~:/?#[\]@!$&*+,;=]+|dtn:\/\/[a-zA-Z0-9\-._~:/?#[\]@!$&*+,;=]+)/g,
    (match) => {
      let url = match;
      let trailing = '';
      while (/[.,;:!?)]$/.test(url)) {
        if (url.endsWith(')')) {
          const openCount = (url.match(/\(/g) || []).length;
          const closeCount = (url.match(/\)/g) || []).length;
          if (closeCount <= openCount) {
            break;
          }
        }
        trailing = url.slice(-1) + trailing;
        url = url.slice(0, -1);
      }
      const safeUrl = escapeAttribute(url);
      return `<a href="${safeUrl}" target="_blank" rel="noopener noreferrer" class="text-emerald-600 dark:text-emerald-400 underline hover:opacity-85">${escapeHtml(url)}</a>${trailing}`;
    }
  );

  // 9. Bullet lists
  formatted = formatted.replace(/^[-*+] (.*$)/gm, '<li class="ml-4 list-disc">$1</li>');

  // 10. Line breaks
  formatted = formatted.replace(/\n/g, '<br/>');

  // 11. Restore placeholders in reverse order
  for (let i = placeholders.length - 1; i >= 0; i--) {
    const item = placeholders[i];
    formatted = formatted.replaceAll(item.token, item.html);
  }

  return formatted;
}
