// Tests of the AI answer renderer (src/lib/markdown.ts): model text stays inert, Markdown maps to
// the expected tags. Run by `npm run check` (`node --test`; Node strips the types).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { escapeHtml, MAX_MARKDOWN_CHARS, renderMarkdown } from '../src/lib/markdown.ts'

/** Every tag the renderer may emit: fixed names, no attributes but its own class names. */
const ALLOWED = /^<\/?(h3|h4|h5|h6|p|ul|ol|li|pre|code|strong|em|del|blockquote|hr|table|thead|tbody|tr|th|td|span)( class="md-[a-z]+")?>$/

function assertOnlyAllowedTags(html: string) {
  for (const tag of html.match(/<[^>]*>/g) ?? []) assert.match(tag, ALLOWED, `unexpected tag ${tag} in ${html}`)
}

const HOSTILE = [
  '<script>alert(1)</script>',
  '<img src=x onerror=alert(1)>',
  '<a href="javascript:alert(1)">x</a>',
  '<svg/onload=alert(1)>',
  '<iframe srcdoc="<script>alert(1)</script>"></iframe>',
  '"><b onmouseover="alert(1)">',
  '<style>body{display:none}</style>',
  '&lt;script&gt;',
]

test('raw HTML in model text is escaped everywhere', () => {
  for (const h of HOSTILE) {
    const shapes = [h, `# ${h}`, `- ${h}`, `1. ${h}`, `> ${h}`, `**${h}**`, `*${h}*`, `\`${h}\``, `\`\`\`\n${h}\n\`\`\``, `| a |\n|---|\n| ${h} |`, `[${h}](${h})`, `![${h}](x)`]
    for (const md of shapes) {
      const html = renderMarkdown(md)
      assertOnlyAllowedTags(html)
      assert.ok(!html.includes('<' + h.slice(1, 4)), `${md} → ${html}`)
    }
  }
  assert.equal(renderMarkdown('<script>alert(1)</script>'), '<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>')
  assert.equal(renderMarkdown('<img src=x onerror=alert(1)>'), '<p>&lt;img src=x onerror=alert(1)&gt;</p>')
  // Already-escaped entities are not decoded back into markup.
  assert.equal(renderMarkdown('&lt;script&gt;'), '<p>&amp;lt;script&amp;gt;</p>')
})

test('code cannot break out of its block', () => {
  const html = renderMarkdown('```html\n</code></pre><script>alert(1)</script>\n```')
  assert.equal(html, '<pre class="md-code"><code>&lt;/code&gt;&lt;/pre&gt;&lt;script&gt;alert(1)&lt;/script&gt;</code></pre>')
  assert.equal(renderMarkdown('Use `<b>` here'), '<p>Use <code>&lt;b&gt;</code> here</p>')
})

test('links and images are shown as text, never as anchors or requests', () => {
  assert.equal(renderMarkdown('[docs](https://example.com/a?b=1&c=2)'), '<p><span class="md-link">docs</span> <span class="md-url">(https://example.com/a?b=1&amp;c=2)</span></p>')
  assert.equal(renderMarkdown('[click](javascript:alert(1))'), '<p><span class="md-link">click</span> <span class="md-url">(javascript:alert(1))</span></p>')
  assert.equal(renderMarkdown('![chart](https://tracker.example/p.png)'), '<p><span class="md-image">[image: chart]</span></p>')
  assert.equal(renderMarkdown('<https://example.com>'), '<p><span class="md-url">https://example.com</span></p>')
  const html = renderMarkdown('[x](https://example.com "title") <https://a.example/?q=<b>>')
  assert.ok(!html.includes('href') && !html.includes('<a') && !html.includes('src='), html)
})

test('quotes in emphasis cannot become attributes', () => {
  assert.equal(renderMarkdown('**" onmouseover="alert(1)**'), '<p><strong>&quot; onmouseover=&quot;alert(1)</strong></p>')
  assert.equal(escapeHtml(`<>&"'`), '&lt;&gt;&amp;&quot;&#39;')
})

test('headings, paragraphs, emphasis and code', () => {
  assert.equal(
    renderMarkdown('# Title\n\nSome **bold** and *italic* and _also_ and ~~gone~~ `x`.'),
    '<h3>Title</h3><p>Some <strong>bold</strong> and <em>italic</em> and <em>also</em> and <del>gone</del> <code>x</code>.</p>',
  )
  assert.equal(renderMarkdown('## C# notes ##'), '<h4>C# notes</h4>')
  assert.equal(renderMarkdown('line one\nline two\n\nnext'), '<p>line one\nline two</p><p>next</p>')
  assert.equal(renderMarkdown('```dax\nSUM ( Sales[Amount] )\n```'), '<pre class="md-code"><code>SUM ( Sales[Amount] )</code></pre>')
  assert.equal(renderMarkdown('---'), '<hr>')
  assert.equal(renderMarkdown('> quoted *text*'), '<blockquote><p>quoted <em>text</em></p></blockquote>')
})

test('identifiers with underscores and stars stay literal', () => {
  assert.equal(renderMarkdown('Use Sales_Amount_Total and 2 * 3 * 4'), '<p>Use Sales_Amount_Total and 2 * 3 * 4</p>')
  assert.equal(renderMarkdown('\\*not italic\\*'), '<p>*not italic*</p>')
  assert.equal(renderMarkdown('Sales[Amount]'), '<p>Sales[Amount]</p>')
})

test('lists, nested lists and tables', () => {
  assert.equal(renderMarkdown('- a\n- b\n  - c\n- d'), '<ul><li>a</li><li>b<ul><li>c</li></ul></li><li>d</li></ul>')
  assert.equal(renderMarkdown('1. one\n2. two\n\ntext'), '<ol><li>one</li><li>two</li></ol><p>text</p>')
  assert.equal(renderMarkdown('- item\n  continued'), '<ul><li>item\ncontinued</li></ul>')
  assert.equal(
    renderMarkdown('| Name | Value |\n|---|---:|\n| `x` | 1 |'),
    '<table class="md-table"><thead><tr><th>Name</th><th>Value</th></tr></thead><tbody><tr><td><code>x</code></td><td>1</td></tr></tbody></table>',
  )
})

test('hostile and huge input stays fast and bounded', () => {
  for (const md of ['*a '.repeat(50_000), '['.repeat(100_000), '_'.repeat(100_000), '`'.repeat(100_000), '> '.repeat(10_000) + 'x', '- '.repeat(20_000) + 'x', '**'.repeat(50_000) + 'x']) {
    const started = Date.now()
    const html = renderMarkdown(md)
    assert.ok(Date.now() - started < 2000, `slow on ${md.slice(0, 12)}…`)
    assertOnlyAllowedTags(html)
  }
  const long = renderMarkdown('x'.repeat(MAX_MARKDOWN_CHARS + 10))
  assert.ok(long.endsWith('<p class="md-cut">… (10 more characters not shown)</p>'), long.slice(-80))
})
