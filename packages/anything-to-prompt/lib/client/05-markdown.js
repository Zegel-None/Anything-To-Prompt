function stripMarkdownForLip(raw) {
  if (!raw) return '';
  let s = String(raw).replace(/\r\n?/g, '\n');
  // Strip fenced code block markers while keeping short inline content readable
  s = s.replace(/```[^\n]*\n?([\s\S]*?)```/g, (_, inner) => {
    const clean = String(inner || '').replace(/\s+/g, ' ').trim();
    return clean.length > 64 ? ' ' + clean.slice(0, 64) + '… ' : ' ' + clean + ' ';
  });
  s = s.replace(/```[^\n]*/g, ' ');
  // Replace markdown images and links with their visible labels (linear-time, zero ReDoS backtracking)
  s = s.replace(/!\[([^\]]*)\]\(([^()\n]*(?:\([^()\n]*\)[^()\n]*)*)\)/g, '$1');
  s = s.replace(/\[([^\]]+)\]\(([^()\n]*(?:\([^()\n]*\)[^()\n]*)*)\)/g, '$1');
  // Strip ATX headings, horizontal rules, table separator lines, blockquote markers, alerts, list markers
  s = s.replace(/^\s*([-*_])(?:\s*\1){2,}\s*$/gm, ' ');
  s = s.replace(/^\s*\|?\s*:?-{2,}:?\s*(?:\|\s*:?-{2,}:?\s*)+\|?\s*$/gm, ' ');
  s = s.replace(/^\s{0,3}#{1,6}\s+/gm, '');
  s = s.replace(/^\s*>\s?(?:\[!(?:NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*)?/gim, '');
  s = s.replace(/^\s*(?:[-*+]|\d+\.)\s+(?:\[[ xX]\]\s+)?/gm, '· ');
  s = s.replace(/\|/g, ' ');
  // Strip inline formatting markers
  s = s.replace(/`{1,2}([^`]+)`{1,2}/g, '$1');
  s = s.replace(/(\*\*|__|~~)([\s\S]*?)\1/g, '$2');
  s = s.replace(/(^|[\s(（【\[])\*([^*\n]+)\*(?=$|[\s.,;:!?)\]）】。，；：])/g, '$1$2');
  return s.replace(/\s+/g, ' ').trim();
}

function openMarkdownLink(href, e) {
  if (e) {
    e.preventDefault();
    e.stopPropagation();
  }
  const target = String(href || '').trim();
  if (!target) return;
  if (/^file:\/\//i.test(target)) {
    let localPath = target.replace(/^file:\/\//i, '').replace(/#.*$/, '');
    if (/^\/[a-zA-Z]:/.test(localPath)) localPath = localPath.slice(1);
    try {
      localPath = decodeURIComponent(localPath);
    } catch {}
    if (/^[a-zA-Z]:\//.test(localPath)) {
      localPath = localPath.replace(/\//g, '\\');
    }
    tauriInvoke('quick_search_open_path', { path: localPath }).catch(() => {});
    return;
  }
  if (/^https?:\/\//i.test(target) || /^mailto:/i.test(target)) {
    try {
      window.open(target, '_blank', 'noopener,noreferrer');
    } catch {}
  }
}

function MdCodeBlock({ lang, code }) {
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(
    (e) => {
      e.stopPropagation();
      if (typeof navigator !== 'undefined' && navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard
          .writeText(code)
          .then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1400);
          })
          .catch(() => {});
      }
    },
    [code],
  );
  return h(
    'div',
    { 'data-dsb-md-pre-wrap': '' },
    h(
      'div',
      { 'data-dsb-md-pre-head': '' },
      h(
        'span',
        {
          style: {
            fontFamily: 'ui-monospace, "Cascadia Code", Consolas, monospace',
            textTransform: 'lowercase',
          },
        },
        lang || 'code',
      ),
      h(
        'button',
        {
          type: 'button',
          'data-dsb-md-copy': '',
          onClick: handleCopy,
        },
        copied ? '已复制 ✓' : '复制',
      ),
    ),
    h('pre', null, h('code', { 'data-dsb-md-codeblock': '' }, code)),
  );
}

function findClosingMarkerSkippingCode(text, startIdx, marker) {
  const mLen = marker.length;
  let j = startIdx;
  while (j <= text.length - mLen) {
    if (text[j] === '\\') {
      j += 2;
      continue;
    }
    if (text[j] === '`') {
      let ticks = 1;
      while (text[j + ticks] === '`') ticks += 1;
      const endTick = text.indexOf('`'.repeat(ticks), j + ticks);
      if (endTick !== -1) {
        j = endTick + ticks;
        continue;
      }
    }
    if (text.slice(j, j + mLen) === marker) {
      if (mLen === 1 && text[j + 1] === marker) {
        j += 2;
        continue;
      }
      if (j > startIdx && !/\s/.test(text[j - 1])) {
        return j;
      }
    }
    j += 1;
  }
  return -1;
}

function renderMarkdownInline(text, keyPrefix, depth) {
  const d = depth || 0;
  const src = String(text || '');
  if (!src) return null;
  const out = [];
  let buf = '';
  let tok = 0;
  let i = 0;
  const len = src.length;
  const flush = () => {
    if (buf) {
      out.push(buf);
      buf = '';
    }
  };

  while (i < len) {
    const ch = src[i];

    // Escaped punctuation
    if (ch === '\\' && i + 1 < len && /[\\`*_[\]()~#>-]/.test(src[i + 1])) {
      buf += src[i + 1];
      i += 2;
      continue;
    }

    // Line break inside paragraph
    if (ch === '\n') {
      flush();
      out.push(h('br', { key: keyPrefix + '-br-' + tok++ }));
      i += 1;
      continue;
    }

    // Inline code: `code` or ``code``
    if (ch === '`') {
      let ticks = 1;
      while (src[i + ticks] === '`' && ticks < 3) ticks += 1;
      const delim = '`'.repeat(ticks);
      const closeIdx = src.indexOf(delim, i + ticks);
      if (closeIdx !== -1 && closeIdx > i + ticks) {
        let inner = src.slice(i + ticks, closeIdx);
        if (ticks > 1 && inner.startsWith(' ') && inner.endsWith(' ') && inner.trim()) {
          inner = inner.slice(1, -1);
        }
        flush();
        out.push(h('code', { key: keyPrefix + '-c-' + tok++ }, inner));
        i = closeIdx + ticks;
        continue;
      }
    }

    // Markdown link: [label](url)
    if (ch === '[') {
      let j = i + 1;
      let bracketDepth = 1;
      while (j < len && bracketDepth > 0) {
        if (src[j] === '\\') {
          j += 2;
          continue;
        }
        if (src[j] === '`') {
          const ct = src.indexOf('`', j + 1);
          if (ct !== -1) {
            j = ct + 1;
            continue;
          }
        }
        if (src[j] === '[') bracketDepth += 1;
        else if (src[j] === ']') bracketDepth -= 1;
        j += 1;
      }
      if (bracketDepth === 0 && src[j] === '(') {
        let k = j + 1;
        let parenDepth = 1;
        const allowSpaces = src.slice(j + 1, j + 8).toLowerCase() === 'file://';
        while (k < len && parenDepth > 0) {
          if (src[k] === '\\') {
            k += 2;
            continue;
          }
          if (src[k] === '(') parenDepth += 1;
          else if (src[k] === ')') parenDepth -= 1;
          else if (allowSpaces ? /[\r\n]/.test(src[k]) : /\s/.test(src[k])) break;
          k += 1;
        }
        if (parenDepth === 0) {
          const label = src.slice(i + 1, j - 1);
          const href = src.slice(j + 1, k - 1).trim();
          if (/^(https?:\/\/|file:\/\/|mailto:)/i.test(href)) {
            flush();
            const linkKey = keyPrefix + '-a-' + tok++;
            out.push(
              h(
                'a',
                {
                  key: linkKey,
                  href,
                  title: href,
                  target: '_blank',
                  rel: 'noopener noreferrer',
                  onClick: (e) => openMarkdownLink(href, e),
                },
                d < 3 ? renderMarkdownInline(label, linkKey, d + 1) : label,
              ),
            );
            i = k;
            continue;
          }
        }
      }
    }

    // Bold: **text** or __text__
    if (d < 3 && ((ch === '*' && src[i + 1] === '*') || (ch === '_' && src[i + 1] === '_'))) {
      const marker = ch + ch;
      const prevWord = i > 0 && /[A-Za-z0-9]/.test(src[i - 1]);
      if (!(ch === '_' && prevWord) && i + 2 < len && !/\s/.test(src[i + 2])) {
        const closeIdx = findClosingMarkerSkippingCode(src, i + 2, marker);
        if (closeIdx !== -1) {
          const inner = src.slice(i + 2, closeIdx);
          flush();
          const bKey = keyPrefix + '-b-' + tok++;
          out.push(h('strong', { key: bKey }, renderMarkdownInline(inner, bKey, d + 1)));
          i = closeIdx + 2;
          continue;
        }
      }
    }

    // Strikethrough: ~~text~~
    if (d < 3 && ch === '~' && src[i + 1] === '~' && i + 2 < len && !/\s/.test(src[i + 2])) {
      const closeIdx = findClosingMarkerSkippingCode(src, i + 2, '~~');
      if (closeIdx !== -1) {
        const inner = src.slice(i + 2, closeIdx);
        flush();
        const sKey = keyPrefix + '-s-' + tok++;
        out.push(h('del', { key: sKey }, renderMarkdownInline(inner, sKey, d + 1)));
        i = closeIdx + 2;
        continue;
      }
    }

    // Italic: *text* (single asterisk, not **)
    if (d < 3 && ch === '*' && src[i + 1] !== '*' && i + 1 < len && !/\s/.test(src[i + 1])) {
      const closeIdx = findClosingMarkerSkippingCode(src, i + 1, '*');
      if (closeIdx !== -1 && src.slice(i + 1, closeIdx).indexOf('\n') === -1) {
        const inner = src.slice(i + 1, closeIdx);
        flush();
        const eKey = keyPrefix + '-em-' + tok++;
        out.push(h('em', { key: eKey }, renderMarkdownInline(inner, eKey, d + 1)));
        i = closeIdx + 1;
        continue;
      }
    }

    // Bare URL: https://... or http://...
    if (
      (ch === 'h' || ch === 'H') &&
      (src.slice(i, i + 8).toLowerCase() === 'https://' ||
        src.slice(i, i + 7).toLowerCase() === 'http://') &&
      (i === 0 || !/[A-Za-z0-9=_"']/.test(src[i - 1]))
    ) {
      let end = i;
      while (end < len && !/[\s<>"'`\u3000-\u303f\uff00-\uffef]/.test(src[end])) {
        end += 1;
      }
      let url = src.slice(i, end);
      while (url.length > 8 && /[.,;:!?\]}>]$/.test(url)) {
        url = url.slice(0, -1);
      }
      while (url.endsWith(')') && (url.match(/\(/g) || []).length < (url.match(/\)/g) || []).length) {
        url = url.slice(0, -1);
      }
      if (url.length > 8) {
        flush();
        out.push(
          h(
            'a',
            {
              key: keyPrefix + '-u-' + tok++,
              href: url,
              title: url,
              target: '_blank',
              rel: 'noopener noreferrer',
              onClick: (e) => openMarkdownLink(url, e),
            },
            url,
          ),
        );
        i += url.length;
        continue;
      }
    }

    buf += ch;
    i += 1;
  }
  flush();
  return out;
}

function parseMdTableRow(line) {
  let s = String(line || '').trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|')) s = s.slice(0, -1);
  const cells = [];
  let cur = '';
  let inCode = false;
  for (let i = 0; i < s.length; i += 1) {
    if (s[i] === '\\' && s[i + 1] === '|') {
      cur += '|';
      i += 1;
      continue;
    }
    if (s[i] === '`') inCode = !inCode;
    if (s[i] === '|' && !inCode) {
      cells.push(cur.trim());
      cur = '';
      continue;
    }
    cur += s[i];
  }
  cells.push(cur.trim());
  return cells;
}

function isMdTableSep(line) {
  return /^\s*\|?\s*:?-{2,}:?\s*(?:\|\s*:?-{2,}:?\s*)+\|?\s*$/.test(String(line || ''));
}

function isMdBlockBoundary(line, nextLine) {
  if (!line || !line.trim()) return true;
  if (/^\s*(`{3,}|~{3,})/.test(line)) return true;
  if (/^\s{0,3}#{1,4}\s+/.test(line)) return true;
  if (/^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line)) return true;
  if (/^\s*>\s?/.test(line)) return true;
  if (/^\s*(?:[-*+]|\d+\.)\s+/.test(line)) return true;
  if (line.indexOf('|') !== -1 && nextLine && isMdTableSep(nextLine)) return true;
  return false;
}

function renderMdListNodes(items, baseKey) {
  if (!items || items.length === 0) return null;
  const nodes = [];
  let idx = 0;
  let groupIdx = 0;
  while (idx < items.length) {
    const listType = items[idx].type;
    const baseIndent = items[idx].indent;
    const group = [];
    let hasTasks = false;
    while (idx < items.length && items[idx].indent === baseIndent && items[idx].type === listType) {
      const item = items[idx];
      idx += 1;
      const nested = [];
      while (idx < items.length && items[idx].indent > baseIndent) {
        nested.push(items[idx]);
        idx += 1;
      }
      if (item.checked !== null) hasTasks = true;
      group.push({ item, nested });
    }
    const gKey = baseKey + '-l' + groupIdx++;
    nodes.push(
      h(
        listType,
        {
          key: gKey,
          'data-dsb-md-task-list': hasTasks ? '1' : undefined,
        },
        group.map(({ item, nested }, liIdx) => {
          const liKey = gKey + '-i' + liIdx;
          return h(
            'li',
            { key: liKey },
            item.checked !== null
              ? h(
                  'span',
                  {
                    'data-dsb-md-check': item.checked ? '1' : '0',
                    'aria-hidden': 'true',
                  },
                  item.checked ? '✓' : '',
                )
              : null,
            renderMarkdownInline(item.text, liKey, 0),
            nested.length > 0 ? renderMdListNodes(nested, liKey) : null,
          );
        }),
      ),
    );
  }
  return nodes;
}

function renderMarkdownBlocks(rawText) {
  const normalized = String(rawText || '').replace(/\r\n?/g, '\n');
  if (!normalized.trim()) return null;
  const lines = normalized.split('\n');
  const blocks = [];
  let i = 0;
  let bIdx = 0;

  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i += 1;
      continue;
    }

    // 1. Fenced code block (``` or ~~~)
    const fenceMatch = line.match(/^\s*(`{3,}|~{3,})(.*)$/);
    if (fenceMatch) {
      const fenceChar = fenceMatch[1][0];
      const fenceLen = fenceMatch[1].length;
      const lang = String(fenceMatch[2] || '').trim().split(/\s+/)[0] || '';
      i += 1;
      const codeLines = [];
      const closeRe = new RegExp('^\\s*' + fenceChar + '{' + fenceLen + ',}\\s*$');
      while (i < lines.length && !closeRe.test(lines[i])) {
        codeLines.push(lines[i]);
        i += 1;
      }
      if (i < lines.length) i += 1;
      blocks.push(
        h(MdCodeBlock, {
          key: 'md-code-' + bIdx++,
          lang,
          code: codeLines.join('\n'),
        }),
      );
      continue;
    }

    // 2. ATX Heading (# to ####)
    const headMatch = line.match(/^\s{0,3}(#{1,4})\s+(.+?)\s*#*\s*$/);
    if (headMatch) {
      const level = headMatch[1].length;
      const hKey = 'md-h-' + bIdx++;
      blocks.push(h('h' + level, { key: hKey }, renderMarkdownInline(headMatch[2], hKey, 0)));
      i += 1;
      continue;
    }

    // 3. Horizontal rule (---, ***, ___)
    if (/^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line)) {
      blocks.push(h('hr', { key: 'md-hr-' + bIdx++ }));
      i += 1;
      continue;
    }

    // 4. Blockquote (> ... and GitHub [!NOTE]/[!TIP]/[!IMPORTANT]/[!WARNING]/[!CAUTION] callouts)
    if (/^\s*>\s?/.test(line)) {
      const qLines = [];
      while (i < lines.length && /^\s*>\s?/.test(lines[i])) {
        qLines.push(lines[i].replace(/^\s*>\s?/, ''));
        i += 1;
      }
      const qKey = 'md-bq-' + bIdx++;
      let alertLabel = null;
      if (qLines.length > 0) {
        const alertMatch = qLines[0].match(/^\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*(.*)$/i);
        if (alertMatch) {
          alertLabel = alertMatch[1].toUpperCase();
          if (alertMatch[2]) qLines[0] = alertMatch[2];
          else qLines.shift();
        }
      }
      blocks.push(
        h(
          'blockquote',
          { key: qKey },
          alertLabel ? h('div', { 'data-dsb-md-alert-label': '' }, alertLabel) : null,
          renderMarkdownBlocks(qLines.join('\n')),
        ),
      );
      continue;
    }

    // 5. GFM Table
    if (line.indexOf('|') !== -1 && i + 1 < lines.length && isMdTableSep(lines[i + 1])) {
      const headers = parseMdTableRow(line);
      const aligns = parseMdTableRow(lines[i + 1]).map((cell) => {
        if (cell.startsWith(':') && cell.endsWith(':')) return 'center';
        if (cell.endsWith(':')) return 'right';
        return 'left';
      });
      i += 2;
      const rows = [];
      while (
        i < lines.length &&
        lines[i].trim() &&
        lines[i].indexOf('|') !== -1 &&
        !/^\s*(`{3,}|~{3,})/.test(lines[i])
      ) {
        rows.push(parseMdTableRow(lines[i]));
        i += 1;
      }
      const tKey = 'md-tbl-' + bIdx++;
      blocks.push(
        h(
          'div',
          { key: tKey, 'data-dsb-md-table-wrap': '' },
          h(
            'table',
            null,
            h(
              'thead',
              null,
              h(
                'tr',
                null,
                headers.map((cell, cIdx) =>
                  h(
                    'th',
                    {
                      key: tKey + '-th-' + cIdx,
                      style: aligns[cIdx] && aligns[cIdx] !== 'left' ? { textAlign: aligns[cIdx] } : undefined,
                    },
                    renderMarkdownInline(cell, tKey + '-th-' + cIdx, 0),
                  ),
                ),
              ),
            ),
            rows.length > 0
              ? h(
                  'tbody',
                  null,
                  rows.map((row, rIdx) =>
                    h(
                      'tr',
                      { key: tKey + '-r-' + rIdx },
                      headers.map((_, cIdx) =>
                        h(
                          'td',
                          {
                            key: tKey + '-r-' + rIdx + '-c-' + cIdx,
                            style:
                              aligns[cIdx] && aligns[cIdx] !== 'left'
                                ? { textAlign: aligns[cIdx] }
                                : undefined,
                          },
                          renderMarkdownInline(row[cIdx] || '', tKey + '-r-' + rIdx + '-c-' + cIdx, 0),
                        ),
                      ),
                    ),
                  ),
                )
              : null,
          ),
        ),
      );
      continue;
    }

    // 6. Unordered / Ordered / Task Lists (supports nesting)
    if (/^\s*(?:[-*+]|\d+\.)\s+/.test(line)) {
      const listItems = [];
      while (i < lines.length) {
        const cur = lines[i];
        const lm = cur.match(/^(\s*)(?:([-*+])|(\d+)\.)\s+(.*)$/);
        if (lm) {
          const rawIndent = lm[1].replace(/\t/g, '  ').length;
          const indent = Math.floor(rawIndent / 2);
          const type = lm[2] ? 'ul' : 'ol';
          let content = lm[4];
          let checked = null;
          const tm = content.match(/^\[([ xX])\]\s+(.*)$/);
          if (tm) {
            checked = tm[1].toLowerCase() === 'x';
            content = tm[2];
          }
          listItems.push({ indent, type, text: content, checked });
          i += 1;
          continue;
        }
        // Indented continuation line inside previous list item
        if (
          listItems.length > 0 &&
          cur.trim() &&
          /^\s{2,}/.test(cur) &&
          !isMdBlockBoundary(cur, lines[i + 1])
        ) {
          listItems[listItems.length - 1].text += '\n' + cur.trim();
          i += 1;
          continue;
        }
        break;
      }
      const lKey = 'md-list-' + bIdx++;
      blocks.push(h(React.Fragment, { key: lKey }, renderMdListNodes(listItems, lKey)));
      continue;
    }

    // 7. Paragraph
    const pLines = [line];
    i += 1;
    while (i < lines.length && !isMdBlockBoundary(lines[i], lines[i + 1])) {
      pLines.push(lines[i]);
      i += 1;
    }
    const pKey = 'md-p-' + bIdx++;
    blocks.push(h('p', { key: pKey }, renderMarkdownInline(pLines.join('\n'), pKey, 0)));
  }

  return blocks;
}

const MarkdownMessage = React.memo(function MarkdownMessage({ text, accent, streamingNode }) {
  const rendered = useMemo(() => renderMarkdownBlocks(text), [text]);
  return h(
    'div',
    {
      'data-dsb-md': '1',
      style: accent ? { '--dsb-md-accent': accent } : undefined,
    },
    rendered,
    streamingNode ? h('div', { style: { marginTop: '4px' } }, streamingNode) : null,
  );
});

