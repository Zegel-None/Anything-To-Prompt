// @local/anything-to-prompt/lib/index.js — Anything-to-Prompt Host Plugin (v1.0.0)
// 1. Writes instance discovery JSON (<DSH_HOME>/anything-to-prompt/instances/<port>.json) with authenticatedUrl()
// 2. Relays twin state (/anything-to-prompt/events, /anything-to-prompt/state, /anything-to-prompt/sync)

import { mkdirSync, readFileSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const name = 'anything-to-prompt';
export const inject = ['webServer'];

const HERE = dirname(fileURLToPath(import.meta.url));

function readPluginVersion() {
  try {
    const pkg = JSON.parse(readFileSync(join(HERE, '..', 'package.json'), 'utf8'));
    return String(pkg.version || '2.0.0');
  } catch {
    return '2.0.0';
  }
}

const PLUGIN_VERSION = readPluginVersion();

function resolveDshHome() {
  const envHome = process.env.DSH_HOME;
  if (typeof envHome === 'string' && envHome.trim()) {
    return envHome.trim();
  }
  return join(homedir(), '.dsh');
}

function resolveProfileName(ctx) {
  const cfgProfile = ctx?.root?.config?.profile;
  if (typeof cfgProfile === 'string' && cfgProfile.trim()) {
    return cfgProfile.trim();
  }
  const envProfile = process.env.DSH_PROFILE;
  if (typeof envProfile === 'string' && envProfile.trim()) {
    return envProfile.trim();
  }
  const normalizedUrl = HERE.replaceAll('\\', '/');
  const m = normalizedUrl.match(/\/profiles\/([^/]+)\//);
  return m && m[1] ? m[1] : 'web';
}

function parsePortNumber(value) {
  const n = Number(value);
  return Number.isInteger(n) && n > 0 && n <= 65535 ? n : undefined;
}

function parsePortFromUrl(rawUrl) {
  if (typeof rawUrl !== 'string' || !rawUrl.trim()) return undefined;
  try {
    const u = new URL(rawUrl);
    if (u.port) return parsePortNumber(u.port);
    return u.protocol === 'https:' ? 443 : u.protocol === 'http:' ? 80 : undefined;
  } catch {
    return undefined;
  }
}

function resolveWebServerPort(ws) {
  if (!ws || typeof ws !== 'object') return undefined;
  const direct = parsePortNumber(ws.port) ?? parsePortNumber(ws.listenedPort);
  if (direct !== undefined) return direct;
  const fromUrl = parsePortFromUrl(ws.url);
  if (fromUrl !== undefined) return fromUrl;
  try {
    const addr = typeof ws.address === 'function' ? ws.address() : undefined;
    const p = parsePortNumber(addr && addr.port);
    if (p !== undefined) return p;
  } catch {}
  try {
    const srvAddr = ws.server && typeof ws.server.address === 'function' ? ws.server.address() : undefined;
    const p = parsePortNumber(srvAddr && srvAddr.port);
    if (p !== undefined) return p;
  } catch {}
  try {
    if (typeof ws.getOrigin === 'function') {
      const p = parsePortFromUrl(ws.getOrigin());
      if (p !== undefined) return p;
    }
  } catch {}
  return undefined;
}

function sendJson(res, status, payload) {
  res.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Cache-Control': 'no-store',
  });
  res.end(JSON.stringify(payload));
}

function readJsonBody(req, maxBytes = 512 * 1024) {
  return new Promise((resolveBody, rejectBody) => {
    const chunks = [];
    let total = 0;
    req.on('data', (chunk) => {
      total += chunk.length;
      if (total > maxBytes) {
        req.destroy();
        rejectBody(new Error('Payload too large'));
        return;
      }
      chunks.push(chunk);
    });
    req.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf8').trim();
      if (!raw) {
        resolveBody({});
        return;
      }
      try {
        resolveBody(JSON.parse(raw));
      } catch (err) {
        rejectBody(err);
      }
    });
    req.on('error', rejectBody);
  });
}

function isSameOriginHeader(req) {
  const origin = req.headers.origin;
  if (typeof origin !== 'string' || !origin.trim()) return true;
  const host = req.headers.host;
  if (typeof host !== 'string' || !host.trim()) return false;
  try {
    const originUrl = new URL(origin);
    return originUrl.host === host;
  } catch {
    return false;
  }
}

const twinState = {
  seq: 1,
  updatedAt: Date.now(),
  sourceClientId: '',
  dshOrigin: '',
  sessionPath: '/',
  sessionId: '',
  reason: 'route',
  draft: '',
  draftSessionId: '',
  agentName: '小张',
  widthPx: 760,
  expandedHeightPx: 280,
  desiredHeightPx: 146,
  drawerExpanded: false,
  popupOpen: false,
  pinned: false,
  sseClients: new Set(),
};

function snapshotPayload() {
  return {
    seq: twinState.seq,
    sourceClientId: twinState.sourceClientId,
    sessionPath: twinState.sessionPath,
    sessionId: twinState.sessionId,
    reason: twinState.reason,
    draft: twinState.draft,
    draftSessionId: twinState.draftSessionId,
    agentName: twinState.agentName,
    widthPx: twinState.widthPx,
    expandedHeightPx: twinState.expandedHeightPx,
    desiredHeightPx: twinState.desiredHeightPx,
    pinned: twinState.pinned,
  };
}

function broadcastTwinEvent(eventName, payload) {
  const frame = 'event: ' + eventName + '\ndata: ' + JSON.stringify(payload) + '\n\n';
  for (const clientRes of twinState.sseClients) {
    try {
      clientRes.write(frame);
    } catch {
      twinState.sseClients.delete(clientRes);
    }
  }
}

export function apply(ctx) {
  // 0. Instance discovery file lifecycle
  let discoveryFilePath = null;
  let loggedWsFields = false;

  const removeDiscoveryFile = () => {
    if (!discoveryFilePath) return;
    try {
      const existing = JSON.parse(readFileSync(discoveryFilePath, 'utf8'));
      if (existing && existing.pid === process.pid) {
        unlinkSync(discoveryFilePath);
      }
    } catch {}
    discoveryFilePath = null;
  };

  const tryWriteDiscoveryFile = () => {
    if (discoveryFilePath) return true;
    const ws = ctx.webServer;
    const port = resolveWebServerPort(ws);
    let conn = null;
    try {
      conn = ctx.get?.('connection') ?? null;
    } catch {}

    if (!loggedWsFields && ws) {
      loggedWsFields = true;
      const keys = Object.keys(ws);
      console.info(
        `[anything-to-prompt] webServer inspected: port=${String(port)} keys=[${keys.join(', ')}] hasConnection=${Boolean(conn && typeof conn.authenticatedUrl === 'function')}`,
      );
    }

    if (port === undefined || !conn || typeof conn.authenticatedUrl !== 'function') {
      return false;
    }

    const baseUrl = `http://127.0.0.1:${port}/`;
    const authUrl = conn.authenticatedUrl(baseUrl);
    const dir = join(resolveDshHome(), 'anything-to-prompt', 'instances');
    const targetPath = join(dir, `${port}.json`);
    const tempPath = join(dir, `${port}.${process.pid}.tmp`);

    const payload = {
      port,
      url: authUrl,
      pid: process.pid,
      profile: resolveProfileName(ctx),
      startedAt: new Date().toISOString(),
      pluginVersion: PLUGIN_VERSION,
    };

    try {
      mkdirSync(dir, { recursive: true });
      writeFileSync(tempPath, JSON.stringify(payload, null, 2) + '\n', 'utf8');
      renameSync(tempPath, targetPath);
      discoveryFilePath = targetPath;
      console.info(`[anything-to-prompt] discovery written: ${targetPath}`);
      return true;
    } catch (err) {
      try {
        unlinkSync(tempPath);
      } catch {}
      console.warn('[anything-to-prompt] failed to write discovery file:', err instanceof Error ? err.message : String(err));
      return false;
    }
  };

  ctx.effect(() => {
    let disposed = false;
    let timer = null;
    let attempts = 0;
    const MAX_ATTEMPTS = 120; // ~18s

    const scheduleCheck = () => {
      if (disposed || discoveryFilePath) return;
      if (tryWriteDiscoveryFile()) return;
      attempts += 1;
      if (attempts >= MAX_ATTEMPTS) {
        const port = resolveWebServerPort(ctx.webServer);
        let hasConn = false;
        try {
          hasConn = Boolean(ctx.get?.('connection')?.authenticatedUrl);
        } catch {}
        console.warn(
          `[anything-to-prompt] discovery skipped after timeout (port=${String(port)}, hasConnection=${String(hasConn)})`,
        );
        return;
      }
      timer = setTimeout(scheduleCheck, 150);
    };

    if (!tryWriteDiscoveryFile()) {
      try {
        ctx.inject?.(['connection'], () => {
          if (!disposed) tryWriteDiscoveryFile();
        });
      } catch {}
      try {
        const loader = ctx.get?.('loader');
        if (loader && typeof loader.await === 'function') {
          loader.await().then(() => {
            if (!disposed) tryWriteDiscoveryFile();
          }).catch(() => {});
        }
      } catch {}
      timer = setTimeout(scheduleCheck, 150);
    }

    const onProcessExit = () => removeDiscoveryFile();
    process.once('exit', onProcessExit);

    return () => {
      disposed = true;
      if (timer) clearTimeout(timer);
      process.off('exit', onProcessExit);
      removeDiscoveryFile();
    };
  }, 'anything-to-prompt:discovery');

  // 1. SSE real-time stream for Twin synchronization
  const offEvents = ctx.webServer.register({
    kind: 'exact',
    path: '/anything-to-prompt/events',
    handler(req, res) {
      res.writeHead(200, {
        'Content-Type': 'text/event-stream; charset=utf-8',
        'Cache-Control': 'no-cache, no-transform',
        Connection: 'keep-alive',
      });
      res.write('event: snapshot\ndata: ' + JSON.stringify(snapshotPayload()) + '\n\n');
      twinState.sseClients.add(res);
      req.on('close', () => {
        twinState.sseClients.delete(res);
      });
    },
  });

  // 2. Pollable state endpoint
  const offState = ctx.webServer.register({
    kind: 'exact',
    path: '/anything-to-prompt/state',
    handler(_req, res) {
      sendJson(res, 200, {
        ok: true,
        version: PLUGIN_VERSION,
        updatedAt: twinState.updatedAt,
        dshOrigin: twinState.dshOrigin,
        drawerExpanded: twinState.drawerExpanded,
        popupOpen: twinState.popupOpen,
        ...snapshotPayload(),
      });
    },
  });

  // 3. Hardened bidirectional live twin state update
  const offSync = ctx.webServer.register({
    kind: 'exact',
    path: '/anything-to-prompt/sync',
    async handler(req, res) {
      if (req.method !== 'POST') {
        sendJson(res, 405, { ok: false, error: 'Method not allowed' });
        return;
      }
      const contentType = String(req.headers['content-type'] || '').toLowerCase();
      if (!contentType.includes('application/json')) {
        sendJson(res, 415, { ok: false, error: 'Content-Type must be application/json' });
        return;
      }
      if (req.headers['x-dsb-client'] !== '1') {
        sendJson(res, 403, { ok: false, error: 'Missing X-DSB-Client header' });
        return;
      }
      if (!isSameOriginHeader(req)) {
        sendJson(res, 403, { ok: false, error: 'Cross-origin request rejected' });
        return;
      }

      try {
        const body = await readJsonBody(req);
        let shouldBroadcast = false;

        if (typeof body.clientId === 'string') {
          twinState.sourceClientId = body.clientId;
        }
        if (typeof body.dshOrigin === 'string' && body.dshOrigin) {
          twinState.dshOrigin = body.dshOrigin.replace(/\/+$/, '');
        }

        const reason = body.reason === 'focus' ? 'focus' : 'route';
        twinState.reason = reason;
        if (reason === 'focus') {
          shouldBroadcast = true;
        }

        if (typeof body.sessionPath === 'string' && body.sessionPath.startsWith('/')) {
          if (twinState.sessionPath !== body.sessionPath) {
            twinState.sessionPath = body.sessionPath;
            shouldBroadcast = true;
          }
        }
        if (typeof body.sessionId === 'string' && twinState.sessionId !== body.sessionId) {
          twinState.sessionId = body.sessionId;
          shouldBroadcast = true;
        }
        if (typeof body.draft === 'string') {
          const nextDraftSessionId =
            typeof body.draftSessionId === 'string'
              ? body.draftSessionId
              : typeof body.sessionId === 'string'
                ? body.sessionId
                : twinState.draftSessionId;
          if (twinState.draft !== body.draft || twinState.draftSessionId !== nextDraftSessionId) {
            twinState.draft = body.draft;
            twinState.draftSessionId = nextDraftSessionId;
            shouldBroadcast = true;
          }
        }
        if (typeof body.agentName === 'string' && body.agentName.trim()) {
          if (twinState.agentName !== body.agentName.trim()) {
            twinState.agentName = body.agentName.trim();
            shouldBroadcast = true;
          }
        }
        if (typeof body.widthPx === 'number' && Number.isFinite(body.widthPx)) {
          const clampedW = Math.max(480, Math.min(1200, Math.round(body.widthPx)));
          if (twinState.widthPx !== clampedW) {
            twinState.widthPx = clampedW;
            shouldBroadcast = true;
          }
        }
        if (typeof body.expandedHeightPx === 'number' && Number.isFinite(body.expandedHeightPx)) {
          twinState.expandedHeightPx = Math.max(160, Math.min(560, Math.round(body.expandedHeightPx)));
        }
        if (typeof body.desiredHeightPx === 'number' && Number.isFinite(body.desiredHeightPx)) {
          twinState.desiredHeightPx = Math.max(56, Math.min(860, Math.round(body.desiredHeightPx)));
        }
        if (typeof body.drawerExpanded === 'boolean') {
          twinState.drawerExpanded = body.drawerExpanded;
        }
        if (typeof body.popupOpen === 'boolean') {
          twinState.popupOpen = body.popupOpen;
        }
        if (typeof body.pinned === 'boolean' && twinState.pinned !== body.pinned) {
          twinState.pinned = body.pinned;
          shouldBroadcast = true;
        }

        if (shouldBroadcast) {
          twinState.seq += 1;
          twinState.updatedAt = Date.now();
          broadcastTwinEvent('twin-sync', snapshotPayload());
        }

        sendJson(res, 200, { ok: true, seq: twinState.seq });
      } catch (err) {
        sendJson(res, 400, {
          ok: false,
          error: String(err && err.message ? err.message : err),
        });
      }
    },
  });

  ctx.effect(() => () => {
    try { offEvents?.(); } catch {}
    try { offState?.(); } catch {}
    try { offSync?.(); } catch {}
  }, 'anything-to-prompt:routes');
}
