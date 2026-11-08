import { test, expect } from '@playwright/test';
import { renderMarkdown, isSafeLinkUrl } from '../src/utils/markdown';

test.describe('Markdown Link & URL rendering', () => {
  test('renders relative file markdown link with trailing punctuation', () => {
    const input = 'Check [dtnprint](src/bin/dtnprint.rs): CLI tool';
    const output = renderMarkdown(input);
    expect(output).toContain('<a href="src/bin/dtnprint.rs" target="_blank" rel="noopener noreferrer" class="text-emerald-600 dark:text-emerald-400 underline hover:opacity-85">dtnprint</a>: CLI tool');
  });

  test('renders web links with optional title', () => {
    const input = '[Hardy](https://example.com "Hardy project")';
    const output = renderMarkdown(input);
    expect(output).toContain('<a href="https://example.com" target="_blank" rel="noopener noreferrer" title="Hardy project" class="text-emerald-600 dark:text-emerald-400 underline hover:opacity-85">Hardy</a>');
  });

  test('renders DTN scheme links', () => {
    const input = '[Station A](dtn://stata/files)';
    const output = renderMarkdown(input);
    expect(output).toContain('<a href="dtn://stata/files" target="_blank" rel="noopener noreferrer" class="text-emerald-600 dark:text-emerald-400 underline hover:opacity-85">Station A</a>');
  });

  test('blocks unsafe javascript and data URIs in markdown links', () => {
    const inputJs = '[Click me](javascript:alert(1))';
    expect(renderMarkdown(inputJs)).toContain('[Click me](unsafe link blocked)');

    const inputData = '[Data](data:text/html,<script>alert(1)</script>)';
    expect(renderMarkdown(inputData)).toContain('[Data](unsafe link blocked)');
  });

  test('preserves code blocks and inline code containing markdown links', () => {
    const inlineInput = 'Run `[dtnprint](src/bin/dtnprint.rs):` now';
    expect(renderMarkdown(inlineInput)).toContain('>[dtnprint](src/bin/dtnprint.rs):</code>');

    const fencedInput = '```rust\nlet x = [dtnprint](src/bin/dtnprint.rs);\n```';
    expect(renderMarkdown(fencedInput)).toContain('<pre class="my-2 p-3 bg-slate-900 text-slate-100 rounded-xl text-xs font-mono overflow-x-auto whitespace-pre-wrap">let x = [dtnprint](src/bin/dtnprint.rs);</pre>');
  });

  test('supports inline formatting inside link label', () => {
    const codeLabel = 'See [`dtnprint`](src/bin/dtnprint.rs): CLI tool';
    const output = renderMarkdown(codeLabel);
    expect(output).toContain('<a href="src/bin/dtnprint.rs" target="_blank" rel="noopener noreferrer" class="text-emerald-600 dark:text-emerald-400 underline hover:opacity-85"><code class="px-1.5 py-0.5 bg-slate-200 dark:bg-slate-700 rounded text-xs font-mono">dtnprint</code></a>: CLI tool');
  });

  test('isSafeLinkUrl correctly evaluates safe and unsafe URLs', () => {
    expect(isSafeLinkUrl('src/bin/dtnprint.rs')).toBe(true);
    expect(isSafeLinkUrl('./path/to/file')).toBe(true);
    expect(isSafeLinkUrl('/api/download')).toBe(true);
    expect(isSafeLinkUrl('https://example.com/test')).toBe(true);
    expect(isSafeLinkUrl('dtn://stata/files')).toBe(true);
    expect(isSafeLinkUrl('mailto:test@example.com')).toBe(true);

    expect(isSafeLinkUrl('javascript:alert(1)')).toBe(false);
    expect(isSafeLinkUrl('data:text/html,bad')).toBe(false);
    expect(isSafeLinkUrl('jav&#x61;script:alert(1)')).toBe(false);
    expect(isSafeLinkUrl('vbscript:msgbox')).toBe(false);
  });
});
