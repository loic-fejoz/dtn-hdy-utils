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

// Validates whether a link URL is safe (http, https, dtn)
export function isSafeLinkUrl(url: string): boolean {
  const trimmed = url.trim();
  const isWebOrDtn = /^(?:https?|dtn):\/\/[a-zA-Z0-9\-._~:/?#[\]@!$&'()*+,;=]+$/i.test(trimmed);
  return isWebOrDtn;
}

export function renderMarkdown(content: string, options: { allowImages?: boolean } = {}): string {
  const images: { placeholder: string; tag: string }[] = [];
  let counter = 0;

  let textToProcess = content;

  if (options.allowImages) {
    // Match ![alt](url)
    textToProcess = textToProcess.replace(/!\[(.*?)\]\((.*?)\)/g, (_match, alt, url) => {
      const cleanUrl = url.trim();
      if (!isSafeImageUrl(cleanUrl)) {
        return `[Image blocked: unsafe URL]`;
      }
      const placeholder = `@@HDYIMGTOKEN${counter++}@@`;
      const safeAlt = escapeHtml(alt || 'Image');
      const safeUrl = escapeAttribute(cleanUrl);
      const tag = `<div class="my-3"><img src="${safeUrl}" alt="${safeAlt}" class="max-h-96 rounded-xl shadow border border-slate-200 dark:border-slate-700 object-contain mx-auto" /><p class="text-center text-xs text-slate-500 dark:text-slate-400 mt-1">${safeAlt}</p></div>`;
      images.push({ placeholder, tag });
      return placeholder;
    });
  }

  // Escape HTML on the body
  let formatted = escapeHtml(textToProcess);

  // Fenced code blocks ```code```
  formatted = formatted.replace(/```([\s\S]*?)```/g, (_match, p1) => {
    return `<pre class="my-2 p-3 bg-slate-900 text-slate-100 rounded-xl text-xs font-mono overflow-x-auto whitespace-pre-wrap">${p1.trim()}</pre>`;
  });

  // Inline code `code`
  formatted = formatted.replace(/`([^`]+)`/g, '<code class="px-1.5 py-0.5 bg-slate-200 dark:bg-slate-700 rounded text-xs font-mono">$1</code>');

  // Headings
  formatted = formatted.replace(/^### (.*$)/gm, '<h3 class="text-base font-bold text-slate-900 dark:text-white mt-3 mb-1">$1</h3>');
  formatted = formatted.replace(/^## (.*$)/gm, '<h2 class="text-lg font-bold text-emerald-600 dark:text-emerald-400 mt-4 mb-2">$1</h2>');
  formatted = formatted.replace(/^# (.*$)/gm, '<h1 class="text-xl font-extrabold text-slate-900 dark:text-white mt-4 mb-2 pb-1 border-b border-slate-200 dark:border-slate-700">$1</h1>');

  // Bold & Italic
  formatted = formatted.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  formatted = formatted.replace(/\*([^*]+)\*/g, '<em>$1</em>');
  formatted = formatted.replace(/_([^_]+)_/g, '<em>$1</em>');

  // Links: match safe URLs ending at whitespace or quote/tag boundary
  formatted = formatted.replace(
    /(https?:\/\/[a-zA-Z0-9\-._~:/?#[\]@!$&*+,;=]+|dtn:\/\/[a-zA-Z0-9\-._~:/?#[\]@!$&*+,;=]+)/g,
    (url) => {
      const safeUrl = escapeAttribute(url);
      return `<a href="${safeUrl}" target="_blank" rel="noopener noreferrer" class="text-emerald-600 dark:text-emerald-400 underline hover:opacity-85">${escapeHtml(url)}</a>`;
    }
  );

  // Bullet lists
  formatted = formatted.replace(/^[-*+] (.*$)/gm, '<li class="ml-4 list-disc">$1</li>');

  // Line breaks
  formatted = formatted.replace(/\n/g, '<br/>');

  // Re-insert safe image tags
  for (const item of images) {
    formatted = formatted.replaceAll(item.placeholder, item.tag);
  }

  return formatted;
}
