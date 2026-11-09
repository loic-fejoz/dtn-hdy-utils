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

test.describe('Markdown Checklist & Task List rendering', () => {
  test('renders unchecked checklist item (- [ ])', () => {
    const input = '- [ ] Deploy station coordinator';
    const output = renderMarkdown(input);
    expect(output).toContain('<input type="checkbox" disabled');
    expect(output).not.toContain('checked disabled');
    expect(output).toContain('<span>Deploy station coordinator</span>');
    expect(output).toContain('list-none');
  });

  test('renders checked checklist items with - [x] and - [X]', () => {
    const inputLower = '- [x] Configure VHF TNC';
    const outputLower = renderMarkdown(inputLower);
    expect(outputLower).toContain('<input type="checkbox" checked disabled');
    expect(outputLower).toContain('<span class="line-through text-slate-400 dark:text-slate-500">Configure VHF TNC</span>');

    const inputUpper = '- [X] Calibrate antenna match';
    const outputUpper = renderMarkdown(inputUpper);
    expect(outputUpper).toContain('<input type="checkbox" checked disabled');
    expect(outputUpper).toContain('<span class="line-through text-slate-400 dark:text-slate-500">Calibrate antenna match</span>');
  });

  test('supports bullet variants (* [ ], + [X], and numbered 1. [ ])', () => {
    const outputStar = renderMarkdown('* [ ] Star unchecked');
    expect(outputStar).toContain('<input type="checkbox" disabled');
    expect(outputStar).not.toContain('checked disabled');
    expect(outputStar).toContain('<span>Star unchecked</span>');

    const outputPlus = renderMarkdown('+ [X] Plus checked');
    expect(outputPlus).toContain('<input type="checkbox" checked disabled');
    expect(outputPlus).toContain('line-through');

    const outputNum = renderMarkdown('1. [ ] Numbered task');
    expect(outputNum).toContain('<input type="checkbox" disabled');
    expect(outputNum).toContain('<span>Numbered task</span>');
  });

  test('supports checkmark symbol variants (- [v], - [+], - [-])', () => {
    const outputV = renderMarkdown('- [v] Validated with v');
    expect(outputV).toContain('<input type="checkbox" checked disabled');
    expect(outputV).toContain('Validated with v');

    const outputPlusMark = renderMarkdown('- [+] Validated with plus');
    expect(outputPlusMark).toContain('<input type="checkbox" checked disabled');

    const outputCancelled = renderMarkdown('- [-] Cancelled objective');
    expect(outputCancelled).toContain('<input type="checkbox" disabled');
    expect(outputCancelled).toContain('line-through');
    expect(outputCancelled).toContain('Cancelled objective');
  });

  test('renders indented sub-checklists with increased margin', () => {
    const input = '- [ ] Main task\n  - [ ] Sub-task indented';
    const output = renderMarkdown(input);
    expect(output).toContain('ml-4 list-none');
    expect(output).toContain('ml-8 list-none');
  });

  test('renders checklist items containing links and inline code', () => {
    const input = '- [ ] Review [dtnprint](src/bin/dtnprint.rs): using `dtnprint -b`';
    const output = renderMarkdown(input);
    expect(output).toContain('<input type="checkbox" disabled');
    expect(output).toContain('<a href="src/bin/dtnprint.rs" target="_blank" rel="noopener noreferrer" class="text-emerald-600 dark:text-emerald-400 underline hover:opacity-85">dtnprint</a>:');
    expect(output).toContain('<code class="px-1.5 py-0.5 bg-slate-200 dark:bg-slate-700 rounded text-xs font-mono">dtnprint -b</code>');
  });

  test('preserves standard bullet lists without checkbox', () => {
    const input = '- Standard bullet item without checklist';
    const output = renderMarkdown(input);
    expect(output).toContain('<li class="ml-4 list-disc">Standard bullet item without checklist</li>');
    expect(output).not.toContain('<input type="checkbox"');
  });
});
