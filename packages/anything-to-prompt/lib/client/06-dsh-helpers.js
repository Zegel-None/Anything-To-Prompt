function isExcludedBlockKind(rawKind) {
  const k = String(rawKind || '').toLowerCase();
  if (!k) return false;
  return (
    k.includes('think') ||
    k.includes('reason') ||
    k.includes('thought') ||
    k.includes('tool') ||
    k.includes('function') ||
    k.includes('status') ||
    k.includes('meta')
  );
}

function stripInlineThinkingTags(text) {
  if (!text) return '';
  return String(text)
    .replace(/<think>[\s\S]*?<\/think>/gi, '')
    .replace(/<thinking>[\s\S]*?<\/thinking>/gi, '')
    .trim();
}

function extractTextFromParts(parts) {
  if (!parts) return '';
  if (typeof parts === 'string') return stripInlineThinkingTags(parts);
  if (!Array.isArray(parts)) return '';
  const buf = [];
  for (const p of parts) {
    if (!p) continue;
    if (typeof p === 'string') {
      buf.push(p);
      continue;
    }
    if (isExcludedBlockKind(p.type || p.kind)) continue;
    if (typeof p.text === 'string' && (p.type === 'text' || p.kind === 'text' || (!p.type && !p.kind))) {
      buf.push(p.text);
    } else if (typeof p.content === 'string' && (!p.type || p.type === 'text') && (!p.kind || p.kind === 'text')) {
      buf.push(p.content);
    }
  }
  return stripInlineThinkingTags(buf.join('\n'));
}

function parseAssistantBlocksList(blocks) {
  const texts = [];
  if (Array.isArray(blocks)) {
    for (const b of blocks) {
      if (!b) continue;
      if (isExcludedBlockKind(b.kind || b.type)) continue;
      if ((b.kind === 'text' || b.type === 'text') && typeof b.text === 'string') {
        const cleaned = stripInlineThinkingTags(b.text);
        if (cleaned) texts.push(cleaned);
      }
    }
  }
  return { text: texts.join('\n\n').trim() };
}

function extractMessagesFromDshProps(props) {
  const result = [];
  if (!props || typeof props !== 'object') return result;

  let chatSnap = null;
  if (typeof props.useConversation === 'function') {
    try {
      chatSnap = props.useConversation((c) => {
        if (!c) return null;
        if (c.views && typeof c.views.get === 'function') return c.views.get('chat');
        return c.chat || null;
      });
    } catch {}
  }
  const session = props.session || null;
  if (!chatSnap && session && session.chat) {
    chatSnap = session.chat;
  }

  if (chatSnap && chatSnap.legacy && Array.isArray(chatSnap.legacy.nodes) && chatSnap.legacy.nodes.length > 0) {
    for (let i = 0; i < chatSnap.legacy.nodes.length; i += 1) {
      const nd = chatSnap.legacy.nodes[i];
      if (!nd) continue;
      if (nd.kind === 'user' || nd.kind === 'steering') {
        const txt = extractTextFromParts(nd.content);
        if (txt) {
          result.push({
            id: String(nd.messageId || 'u-' + i),
            role: 'user',
            text: txt,
            streaming: false,
          });
        }
      } else if (nd.kind === 'assistant') {
        const parsed = parseAssistantBlocksList(nd.blocks);
        if (parsed.text) {
          result.push({
            id: String(nd.messageId || 'a-' + i),
            role: 'assistant',
            text: parsed.text,
            streaming: false,
          });
        }
      }
    }
    if (chatSnap.legacy.partial && Array.isArray(chatSnap.legacy.partial.blocks)) {
      const pp = parseAssistantBlocksList(chatSnap.legacy.partial.blocks);
      if (pp.text) {
        result.push({
          id: 'streaming-partial',
          role: 'assistant',
          text: pp.text,
          streaming: true,
        });
      }
    }
    if (result.length > 0) return result.slice(-40);
  }

  if (
    chatSnap &&
    Array.isArray(chatSnap.order) &&
    chatSnap.nodes &&
    typeof chatSnap.nodes.get === 'function'
  ) {
    for (const key of chatSnap.order) {
      const node = chatSnap.nodes.get(key);
      const d = node && node.data ? node.data : node;
      if (!d) continue;
      const kind = node.kind || d.kind;
      if (isExcludedBlockKind(kind)) continue;
      if (kind === 'user' || kind === 'steering') {
        const txt = extractTextFromParts(d.content ?? d.parts ?? d.text);
        if (txt) {
          result.push({
            id: String(key),
            role: 'user',
            text: txt,
            streaming: false,
          });
        }
      } else if (kind === 'assistant-step' || kind === 'assistant' || Array.isArray(d.blocks)) {
        const parsed = parseAssistantBlocksList(d.blocks);
        if (parsed.text) {
          result.push({
            id: String(key),
            role: 'assistant',
            text: parsed.text,
            streaming: d.status === 'running',
          });
        }
      }
    }
    if (result.length > 0) return result.slice(-40);
  }

  const list =
    session && Array.isArray(session.messages)
      ? session.messages
      : session && Array.isArray(session.items)
        ? session.items
        : session && Array.isArray(session.turns)
          ? session.turns
          : [];
  for (let i = 0; i < list.length; i += 1) {
    const m = list[i];
    if (!m || typeof m !== 'object') continue;
    const rawRole = String(m.role || m.sender || m.author || '').toLowerCase();
    if (rawRole.includes('tool') || rawRole.includes('system') || rawRole.includes('function')) continue;
    const role = rawRole.includes('user') || rawRole.includes('human') ? 'user' : 'assistant';
    const txt = extractTextFromParts(m.content ?? m.parts ?? m.blocks ?? m.text ?? m.message);
    if (txt) {
      result.push({
        id: String(m.id || 'msg-' + i),
        role,
        text: txt,
        streaming: Boolean(m.streaming),
      });
    }
  }
  return result.slice(-40);
}

function postTwinSync(payload) {
  fetch('/anything-to-prompt/sync', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'X-DSB-Client': '1',
    },
    body: JSON.stringify(Object.assign({ clientId: SELF_CLIENT_ID, dshOrigin: window.location.origin }, payload)),
  }).catch(() => {});
}

function renderFolderIcon(size = 14) {
  if (DSHPrimitives && DSHPrimitives.IconFolderCloseRegular) {
    return h(DSHPrimitives.IconFolderCloseRegular, { size, style: { flexShrink: 0, opacity: 0.85 } });
  }
  return h(
    'svg',
    {
      xmlns: 'http://www.w3.org/2000/svg',
      viewBox: '0 0 16 16',
      width: size,
      height: size,
      fill: 'none',
      strokeWidth: 1,
      style: { flexShrink: 0, opacity: 0.85 },
    },
    h('path', {
      d: 'M1.50439 3.11059C1.50439 2.55831 1.95211 2.1106 2.50439 2.1106H5.43389C5.67773 2.1106 5.91318 2.19969 6.09593 2.36113L7.71649 3.79265C7.89924 3.95409 8.1347 4.04319 8.3785 4.04319H13.4958C14.0481 4.04319 14.4958 4.4909 14.4958 5.04319V12.8894C14.4958 13.4417 14.0481 13.8894 13.4958 13.8894H2.50439C1.95211 13.8894 1.50439 13.4417 1.50439 12.8894V4.04319V3.11059Z',
      stroke: 'currentColor',
    }),
    h('path', {
      d: 'M3.63501 7.66614H12.3647',
      stroke: 'currentColor',
    }),
  );
}

function formatRelativeTime(ts) {
  if (!ts) return '';
  const ms = typeof ts === 'number' ? ts : Date.parse(String(ts));
  if (!Number.isFinite(ms)) return '';
  const diffSec = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (diffSec < 60) return '刚刚';
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return diffMin + '分钟前';
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return diffHr + '小时前';
  const diffDay = Math.floor(diffHr / 24);
  if (diffDay < 7) return diffDay + '天前';
  if (diffDay < 30) return Math.floor(diffDay / 7) + '周前';
  return Math.floor(diffDay / 30) + '个月前';
}

function workspaceBasename(p) {
  if (!p || typeof p !== 'string') return '未命名工作区';
  const clean = p.replace(/[\\/]+$/, '');
  const idx = Math.max(clean.lastIndexOf('/'), clean.lastIndexOf('\\'));
  return idx >= 0 ? clean.slice(idx + 1) || clean : clean;
}

function mainSessionIdOfList(listSnap) {
  if (!listSnap || !listSnap.byId) return '';
  const values = Object.values(listSnap.byId);
  for (const row of values) {
    if (row && row.retainedBy && (row.retainedBy.mainView || 0) > 0) {
      return String(row.id || '');
    }
  }
  return '';
}

function sessionDisplayTitle(s) {
  if (!s) return '新对话';
  if (s.blank) return '新对话';
  const t = typeof s.title === 'string' ? s.title.trim() : '';
  return t || '新对话';
}

