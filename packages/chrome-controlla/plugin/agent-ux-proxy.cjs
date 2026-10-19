'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const SIX_TOOLS = ['browser', 'snapshot', 'act', 'workflow', 'extract', 'verify'];
const ROUTE_TTL_MS = 30 * 24 * 60 * 60 * 1000;
const OPEN_RETRY_TTL_MS = 60 * 1000;

const TOOL_DESCRIPTIONS = {
  browser: 'PRIMARY ENTRY POINT. Use first for ordinary Chrome tasks. Route semantic controls first, then a live qualified page tool, then fresh bounded visual evidence; use desktop fallback only when the server advertises its platform capability. Open the most specific known destination directly. For supported common goals, pass goal and Controlla will compile the browser mechanics internally. Do not inspect schemas, source files, or call describe before normal use. If no existing tab is a safe match, use the bounded candidate/reason receipt: open the exact URL in a new Controlla-owned tab, or ask for an exact tab selection when an existing tab is required.',
  snapshot: 'Take one compact semantic observation only when a new decision requires page evidence. Prefer a targeted goal and omit page text when unnecessary. Use context instead of session/tab IDs.',
  act: 'Perform one isolated browser action when a workflow is unnecessary. Prefer workflow for multi-step work. Use context instead of session/tab IDs.',
  workflow: 'Run a known multi-step browser program in one MCP call. Prefer this over repeated snapshot/act loops. Use context instead of session/tab IDs.',
  extract: 'Return bounded structured page data. Use only when structured output is required; do not inspect implementation source to learn its schema.',
  verify: 'Independently verify a browser postcondition when an action receipt does not already prove the requested result.'
};

const COMPACT_FIELDS = {
  browser: ['action', 'url', 'mode', 'ready_when', 'task', 'filename', 'bytes_base64', 'file_path', 'artifact_handle', 'encoding', 'steps', 'reuse_existing'],
  snapshot: ['goal', 'href_hint', 'frame_id', 'max_items', 'max_text_chars', 'max_bytes', 'max_scan_nodes', 'selector'],
  act: ['action', 'reference', 'expected_value', 'value', 'allow_password', 'key', 'outcome', 'timeout_ms', 'typing_mode', 'delay_ms', 'page_tool_name', 'page_tool_input'],
  workflow: ['steps', 'program'],
  extract: ['selector', 'fields', 'max_items', 'max_bytes'],
  verify: ['predicate']
};

const GOAL_SCHEMA = {
  oneOf: [
    {
      type: 'object',
      additionalProperties: false,
      required: ['kind', 'site', 'entity'],
      properties: {
        kind: { const: 'open_newest_item' },
        site: { type: 'string', minLength: 1, maxLength: 255 },
        entity: { type: 'string', minLength: 1, maxLength: 256 },
        item_type: { type: 'string', minLength: 1, maxLength: 64, default: 'story' },
        destination_url: { type: 'string', minLength: 8, maxLength: 4096 },
        link_query: { type: 'string', minLength: 1, maxLength: 256 },
        href_hint: { type: 'string', minLength: 1, maxLength: 512 },
        verify_url_prefix: { type: 'string', minLength: 8, maxLength: 4096 },
        max_candidates: { type: 'integer', minimum: 2, maximum: 32 }
      }
    }
  ]
};

const BUILTIN_ROUTE_PROFILES = new Map([
  [
    'espn.com|open_newest_item|buffalo bills|story',
    {
      site_scope: 'https://www.espn.com',
      destination_url: 'https://www.espn.com/nfl/team/_/name/buf/buffalo-bills',
      link_query: 'Bills',
      href_hint: '/nfl/story/',
      verify_url_prefix: 'https://www.espn.com/nfl/',
      structural_signatures: ['Buffalo Bills', 'NFL'],
      source: 'builtin_verified_profile'
    }
  ]
]);

function clone(value) {
  return value == null ? value : JSON.parse(JSON.stringify(value));
}

function canonicalSite(value) {
  const raw = String(value ?? '').trim().toLowerCase();
  if (!raw) return '';
  try {
    const url = raw.includes('://') ? new URL(raw) : new URL(`https://${raw}`);
    return url.hostname.replace(/^www\./, '');
  } catch {
    return raw.replace(/^https?:\/\//, '').replace(/^www\./, '').split('/')[0];
  }
}

function routeKey(goal) {
  return [
    canonicalSite(goal.site),
    'open_newest_item',
    String(goal.entity ?? '').trim().toLowerCase(),
    String(goal.item_type ?? 'story').trim().toLowerCase()
  ].join('|');
}

function publicHttpUrl(value) {
  try {
    const url = new URL(value);
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) return false;
    const sensitive = ['token', 'password', 'secret', 'session', 'auth', 'key'];
    for (const name of url.searchParams.keys()) {
      if (sensitive.some(word => name.toLowerCase().includes(word))) return false;
    }
    return true;
  } catch {
    return false;
  }
}

function sameOrigin(scope, destination) {
  try {
    return new URL(scope).origin === new URL(destination).origin;
  } catch {
    return false;
  }
}

function cssStringLiteral(value) {
  let escaped = '';
  for (const character of String(value)) {
    const code = character.codePointAt(0);
    if (character === '"' || character === '\\' || code <= 0x1f || code === 0x7f) {
      escaped += `\\${code.toString(16)} `;
    } else {
      escaped += character;
    }
  }
  return `"${escaped}"`;
}

function compactTool(tool) {
  const next = clone(tool);
  next.description = TOOL_DESCRIPTIONS[tool.name] ?? tool.description;
  const source = clone(tool.inputSchema ?? { type: 'object', properties: {} });
  const properties = {};
  for (const field of COMPACT_FIELDS[tool.name] ?? []) {
    if (source.properties?.[field]) properties[field] = source.properties[field];
  }
  properties.context = {
    type: 'string',
    minLength: 4,
    maxLength: 128,
    description: 'Opaque Controlla context handle. Omit when exactly one browser context is active.'
  };
  properties.advanced = {
    type: 'object',
    additionalProperties: true,
    description: 'Rare compatibility escape hatch. Normal tasks should not use this. Read the relevant progressive-disclosure reference before using it.'
  };
  if (tool.name === 'browser') {
    properties.goal = {
      ...GOAL_SCHEMA,
      description: 'Optional high-level goal compiled into the shortest verified browser path. Prefer this for supported common tasks instead of manually orchestrating snapshots and actions.'
    };
    if (properties.action) {
      properties.action.description = 'Browser operation. action=open reuses the current Controlla context when the requested URL is already bound (set reuse_existing=false to force a new tab); otherwise it creates a new target. Context transitions survive window switches and extension focus changes.';
    }
    if (properties.url) {
      properties.url.description = 'Direct HTTP(S) destination when known. Prefer the most specific known destination over a homepage.';
    }
  }
  next.inputSchema = {
    type: 'object',
    additionalProperties: false,
    properties,
    required: (source.required ?? []).filter(name => properties[name] && !['session_id', 'chrome_tab_id', 'idempotency_key'].includes(name)),
    ...(source.$defs ? { $defs: source.$defs } : {}),
    ...(source.definitions ? { definitions: source.definitions } : {})
  };
  return next;
}

function compactTools(tools) {
  const byName = new Map(tools.map(tool => [tool.name, tool]));
  return SIX_TOOLS.filter(name => byName.has(name)).map(name => compactTool(byName.get(name)));
}

function mergeAdvanced(args) {
  const advanced = args?.advanced;
  if (advanced == null) {
    const plain = { ...(args ?? {}) };
    delete plain.context;
    delete plain.goal;
    return plain;
  }
  if (typeof advanced !== 'object' || Array.isArray(advanced)) throw new Error('advanced must be an object');
  const plain = { ...advanced, ...(args ?? {}) };
  delete plain.advanced;
  delete plain.context;
  delete plain.goal;
  return plain;
}

class ContextRegistry {
  constructor() {
    this.contexts = new Map();
  }

  create(sessionId, tabId, metadata = {}) {
    if (!sessionId || !tabId) return null;
    const id = `ctx_${crypto.randomUUID().replaceAll('-', '').slice(0, 20)}`;
    const record = { session_id: String(sessionId), chrome_tab_id: String(tabId), ...clone(metadata) };
    if (Object.keys(metadata).length) record.last_seen_ms = Date.now();
    this.contexts.set(id, record);
    return id;
  }

  resolve(id) {
    if (id) return this.contexts.get(id) ?? null;
    if (this.contexts.size !== 1) return null;
    return this.contexts.values().next().value;
  }

  removeBySession(sessionId) {
    for (const [id, value] of this.contexts) {
      if (value.session_id === sessionId) this.contexts.delete(id);
    }
  }

  size() {
    return this.contexts.size;
  }
}

function normalizedTargetUrl(value) {
  try {
    const url = new URL(String(value));
    url.hash = '';
    return url.toString().replace(/\/$/, '');
  } catch {
    return String(value ?? '').trim().replace(/#.*$/, '').replace(/\/$/, '');
  }
}

function sameTargetUrl(left, right) {
  return Boolean(left && right) && normalizedTargetUrl(left) === normalizedTargetUrl(right);
}

function recoverableContextError(error) {
  const text = String(error?.message ?? error ?? '').toLowerCase();
  return /session (?:is )?not (?:registered|created)|unknown shared session|unknown v3|unknown target|pairing ended|session .* route|tab .* (?:missing|closed|not found)|context probe did not produce a healthy snapshot/.test(text);
}

class OpenKeyCache {
  constructor(now = () => Date.now()) {
    this.now = now;
    this.cache = new Map();
  }

  keyFor(args) {
    const signature = JSON.stringify({
      url: args.url ?? null,
      mode: args.mode ?? null,
      ready_when: args.ready_when ?? null,
      task: args.task ?? null,
      steps: args.steps ?? null
    });
    const now = this.now();
    const existing = this.cache.get(signature);
    if (existing && existing.expires_at_ms > now) return existing.key;
    const digest = crypto.createHash('sha256').update(signature).update(String(now)).digest('hex').slice(0, 24);
    const key = `auto_${digest}`;
    this.cache.set(signature, { key, expires_at_ms: now + OPEN_RETRY_TTL_MS });
    return key;
  }
}

class RouteMemory {
  constructor(stateDir, now = () => Date.now()) {
    this.now = now;
    const root = stateDir || path.join(os.homedir(), '.local', 'state', 'chrome-controlla');
    this.file = path.join(root, 'agent-route-memory.json');
  }

  load() {
    try {
      const parsed = JSON.parse(fs.readFileSync(this.file, 'utf8'));
      return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
    } catch (error) {
      if (error?.code === 'ENOENT') return {};
      return {};
    }
  }

  save(records) {
    fs.mkdirSync(path.dirname(this.file), { recursive: true });
    const temp = `${this.file}.tmp-${process.pid}`;
    fs.writeFileSync(temp, `${JSON.stringify(records, null, 2)}\n`);
    fs.renameSync(temp, this.file);
  }

  resolve(goal) {
    const key = routeKey(goal);
    const records = this.load();
    const record = records[key];
    const now = this.now();
    if (record?.status === 'quarantined') return null;
    if (record?.status === 'qualified' && record.expires_at_ms > now && publicHttpUrl(record.destination_url)) {
      return { ...record, route_key: key, source: 'learned_verified_route', confidence: 1 };
    }
    const builtin = BUILTIN_ROUTE_PROFILES.get(key);
    if (builtin) return { ...builtin, route_key: key, confidence: 1 };
    return null;
  }

  recordSuccess(goal, profile, runId = crypto.randomUUID()) {
    if (!profile?.destination_url || !publicHttpUrl(profile.destination_url)) return;
    if (profile.site_scope && !sameOrigin(profile.site_scope, profile.destination_url)) return;
    const key = routeKey(goal);
    const records = this.load();
    const record = records[key] ?? {
      status: 'candidate',
      destination_url: profile.destination_url,
      site_scope: profile.site_scope ?? new URL(profile.destination_url).origin,
      link_query: profile.link_query ?? goal.link_query ?? goal.entity,
      href_hint: profile.href_hint ?? goal.href_hint ?? null,
      verify_url_prefix: profile.verify_url_prefix ?? goal.verify_url_prefix ?? new URL(profile.destination_url).origin,
      structural_signatures: profile.structural_signatures ?? [goal.entity],
      successful_run_ids: [],
      failed_verifications: 0,
      qualified_at_ms: null,
      expires_at_ms: null
    };
    const runs = new Set(record.successful_run_ids ?? []);
    runs.add(runId);
    record.successful_run_ids = [...runs].slice(-8);
    record.destination_url = profile.destination_url;
    record.failed_verifications = 0;
    if (record.successful_run_ids.length >= 2 || profile.source === 'builtin_verified_profile') {
      record.status = 'qualified';
      record.qualified_at_ms = record.qualified_at_ms ?? this.now();
      record.expires_at_ms = this.now() + ROUTE_TTL_MS;
    }
    records[key] = record;
    this.save(records);
  }

  recordFailure(goal, { structuralDrift = false } = {}) {
    const key = routeKey(goal);
    const records = this.load();
    const record = records[key] ?? { status: 'candidate', failed_verifications: 0, successful_run_ids: [] };
    record.failed_verifications = (record.failed_verifications ?? 0) + 1;
    if (structuralDrift || record.failed_verifications >= 3) record.status = 'quarantined';
    records[key] = record;
    this.save(records);
  }
}

function explicitGoalProfile(goal) {
  if (!goal.destination_url) return null;
  if (!publicHttpUrl(goal.destination_url)) throw new Error('goal.destination_url must be a public HTTP(S) URL without credentials or sensitive query parameters');
  const site = canonicalSite(goal.site);
  if (canonicalSite(goal.destination_url) !== site) throw new Error('goal.destination_url must match goal.site');
  const origin = new URL(goal.destination_url).origin;
  return {
    route_key: routeKey(goal),
    site_scope: origin,
    destination_url: goal.destination_url,
    link_query: goal.link_query ?? goal.entity,
    href_hint: goal.href_hint ?? null,
    verify_url_prefix: goal.verify_url_prefix ?? origin,
    structural_signatures: [goal.entity],
    source: 'explicit_destination',
    confidence: 1
  };
}

function compileNewestWorkflow(goal, profile, mode = 'foreground') {
  const maxCandidates = Math.max(2, Math.min(Number(goal.max_candidates ?? 12), 32));
  const query = goal.link_query ?? profile.link_query ?? goal.entity;
  const hrefHint = goal.href_hint ?? profile.href_hint ?? undefined;
  const verifyPrefix = goal.verify_url_prefix ?? profile.verify_url_prefix ?? new URL(profile.destination_url).origin;
  return [
    {
      kind: 'snapshot',
      goal: {
        query,
        roles: ['link'],
        ...(hrefHint ? { href_hint: hrefHint } : {}),
        exhaustive: true
      },
      max_items: maxCandidates,
      max_text_chars: 0,
      max_scan_nodes: 20000
    },
    {
      kind: 'find_all',
      role: 'link',
      ...(hrefHint ? { href_hint: hrefHint } : {}),
      max_matches: maxCandidates,
      save_as: 'candidates'
    },
    { kind: 'choose', from: 'candidates', key: 'datetime', save_as: 'latest' },
    mode === 'background'
      ? { kind: 'navigate_selected', from: 'latest' }
      : { kind: 'click', reference: '$latest', outcome: { kind: 'navigation' } },
    { kind: 'verify', predicate: { kind: 'url_starts_with', prefix: verifyPrefix } }
  ];
}

function getStructured(result) {
  if (result?.structuredContent && typeof result.structuredContent === 'object') return result.structuredContent;
  for (const item of result?.content ?? []) {
    if (item?.type !== 'text' || typeof item.text !== 'string') continue;
    try {
      const parsed = JSON.parse(item.text);
      if (parsed && typeof parsed === 'object') return parsed;
    } catch {}
  }
  return null;
}

function withStructuredPatch(result, patch) {
  const next = clone(result ?? {});
  const structured = getStructured(next);
  if (structured) {
    next.structuredContent = { ...structured, ...patch };
  } else {
    next.structuredContent = { ...patch };
  }
  next.content = [{ type: 'text', text: JSON.stringify(next.structuredContent) }];
  return next;
}

function failureLike(value) {
  if (!value || typeof value !== 'object') return false;
  const status = String(value.status ?? '').toLowerCase();
  if (['failed', 'unknown', 'not_dispatched', 'unsupported', 'incomplete'].includes(status)) return true;
  return Object.values(value).some(child => child && typeof child === 'object' && failureLike(child));
}

function hasPassedVerify(value) {
  if (!value || typeof value !== 'object') return false;
  if (value.step === 'verify' && value.status === 'passed') return true;
  if (value.status === 'passed' && value.predicate) return true;
  return Object.values(value).some(child => child && typeof child === 'object' && hasPassedVerify(child));
}

function structuralDrift(value) {
  const text = JSON.stringify(value ?? {}).toLowerCase();
  return text.includes('structural drift') || text.includes('no_match') || text.includes('candidate_freshness_incomplete');
}

class AgentUxRuntime {
  constructor({ callTool, stateDir, now = () => Date.now() }) {
    if (typeof callTool !== 'function') throw new Error('callTool is required');
    this.callTool = callTool;
    this.contexts = new ContextRegistry();
    this.openKeys = new OpenKeyCache(now);
    this.routes = new RouteMemory(stateDir, now);
    this.serverTools = [];
  }

  setServerTools(tools) {
    this.serverTools = Array.isArray(tools) ? tools : [];
  }

  publicTools() {
    return compactTools(this.serverTools);
  }

  prepareArgs(name, args) {
    const original = args ?? {};
    const prepared = mergeAdvanced(original);
    let contextId = original.context;
    let resolved = null;
    if (!contextId && name !== 'browser') {
      const contextCount = this.contexts.size();
      if (contextCount > 1) throw new Error('Controlla context is required when multiple browser contexts are active');
      if (contextCount === 1) {
        contextId = 'the sole active context';
        resolved = this.contexts.resolve();
      } else if (![prepared.session_id, prepared.chrome_tab_id].every(value => value != null && String(value).length > 0)) {
        throw new Error('Controlla context or explicit session_id and chrome_tab_id are required');
      }
    }
    if (contextId) {
      resolved ??= this.contexts.resolve(original.context);
      if (!resolved) throw new Error(`unknown Controlla context: ${original.context}`);
      for (const [name, bound, supplied] of [
        ['session_id', resolved.session_id, prepared.session_id],
        ['chrome_tab_id', resolved.chrome_tab_id, prepared.chrome_tab_id]
      ]) {
        if (supplied != null && String(supplied) !== String(bound)) {
          throw new Error(`${name} conflicts with context ${contextId}`);
        }
        prepared[name] = bound;
      }
    }
    if (name === 'browser' && prepared.action === 'open' && !prepared.idempotency_key) {
      prepared.idempotency_key = this.openKeys.keyFor(prepared);
    }
    return prepared;
  }

  rememberContext(result) {
    const value = getStructured(result);
    const sessionId = value?.session_id;
    const tabId = value?.tab_id ?? value?.chrome_tab_id;
    if (!sessionId || !tabId) return { result, context: null };
    const context = this.contexts.create(sessionId, tabId, {
      url: value.url ?? value.document?.url ?? null,
      mode: value.mode ?? null,
      window_id: value.window_id ?? null,
      document_id: value.document_id ?? value.document?.document_id ?? null
    });
    return {
      result: withStructuredPatch(result, {
        context,
        orchestration: {
          external_tool_calls: 1,
          internal_tool_calls: 1,
          schema_discovery_calls: 0,
          repo_reads_required: 0
        }
      }),
      context
    };
  }

  async rebindContext(contextId) {
    const current = this.contexts.resolve(contextId);
    if (!current) return null;
    try {
      const probe = await this.callTool('snapshot', {
        session_id: current.session_id,
        chrome_tab_id: current.chrome_tab_id,
        max_items: 1,
        max_text_chars: 0
      });
      const observed = getStructured(probe);
      const failedStatus = ['unknown', 'failed', 'error', 'not_dispatched', 'incomplete'].includes(String(observed?.status || '').toLowerCase());
      if (!observed || failedStatus || (observed.verified === false && !observed.url)) {
        throw new Error(`Controlla context probe did not produce a healthy snapshot (status=${observed?.status ?? 'missing'})`);
      }
      if (observed?.url) current.url = observed.url;
      current.last_seen_ms = Date.now();
      return current;
    } catch (error) {
      if (!recoverableContextError(error)) throw error;
    }

    // A server/extension restart invalidates the old V3 session, but the
    // Chrome tab can remain alive. Rediscover and pair the exact tab first;
    // only fall back to a unique URL match when Chrome assigned a new tab ID.
    const discovered = getStructured(await this.callTool('browser', {
      action: 'discover_shared_tabs',
      mode: current.mode ?? 'foreground'
    }));
    const snapshot = discovered?.snapshot ?? discovered;
    const tabs = Array.isArray(snapshot?.tabs) ? snapshot.tabs : [];
    let candidate = tabs.find(tab => String(tab.id) === String(current.chrome_tab_id));
    if (!candidate && current.url) {
      const matches = tabs.filter(tab => sameTargetUrl(tab.url, current.url));
      if (matches.length === 1) candidate = matches[0];
      else if (matches.length > 1) throw new Error('Controlla context recovery is ambiguous: multiple tabs still match the bound URL');
    }
    if (!candidate) throw new Error('Controlla context recovery could not find the previously bound tab');
    const paired = getStructured(await this.callTool('browser', {
      action: 'pair_shared',
      mode: current.mode ?? 'foreground',
      host_id: snapshot.host_id,
      target_ids: [String(candidate.id)]
    }));
    const sessionId = paired?.session_id;
    if (!sessionId) throw new Error('Controlla context recovery pairing omitted session_id');
    const accepted = getStructured(await this.callTool('browser', {
      action: 'accept_shared',
      mode: current.mode ?? 'foreground',
      session_id: sessionId
    }));
    if (accepted?.accepted !== true) throw new Error('Controlla context recovery pairing was not accepted');
    current.session_id = String(sessionId);
    current.chrome_tab_id = String(candidate.id);
    current.url = candidate.url ?? current.url;
    current.window_id = candidate.window_id ?? current.window_id;
    current.document_id = candidate.document_id ?? current.document_id;
    current.last_seen_ms = Date.now();
    return current;
  }

  async reuseExistingOpen(args) {
    if (args?.reuse_existing === false || args?.url == null) return null;
    const contextId = args.context ?? (this.contexts.size() === 1 ? [...this.contexts.contexts.keys()][0] : null);
    if (!contextId) return null;
    const current = this.contexts.resolve(contextId);
    if (!current || !sameTargetUrl(current.url, args.url)) return null;
    const rebound = await this.rebindContext(contextId);
    const context = contextId;
    return {
      content: [{ type: 'text', text: JSON.stringify({
        status: 'reused',
        verified: true,
        same_target: true,
        session_id: rebound.session_id,
        tab_id: rebound.chrome_tab_id,
        url: rebound.url,
        mode: rebound.mode,
        context,
        target_reused_after_focus_or_session_change: true,
        orchestration: { external_tool_calls: 1, internal_tool_calls: 0, schema_discovery_calls: 0, repo_reads_required: 0 }
      }) }],
      structuredContent: {
        status: 'reused', verified: true, same_target: true,
        session_id: rebound.session_id, tab_id: rebound.chrome_tab_id,
        url: rebound.url, mode: rebound.mode, context,
        target_reused_after_focus_or_session_change: true,
        orchestration: { external_tool_calls: 1, internal_tool_calls: 0, schema_discovery_calls: 0, repo_reads_required: 0 }
      }
    };
  }

  async runGoal(args) {
    const goal = args.goal;
    if (!goal || goal.kind !== 'open_newest_item') throw new Error('unsupported browser goal');
    const profile = explicitGoalProfile(goal) ?? this.routes.resolve(goal);
    if (!profile) {
      return {
        content: [{ type: 'text', text: JSON.stringify({
          status: 'not_dispatched',
          reason_code: 'no_qualified_direct_route',
          next_action: 'Use the normal browser route once, then allow Controlla to learn the verified destination for future runs.',
          goal
        }) }],
        structuredContent: {
          status: 'not_dispatched',
          reason_code: 'no_qualified_direct_route',
          next_action: 'Use the normal browser route once, then allow Controlla to learn the verified destination for future runs.',
          goal
        }
      };
    }
    const base = mergeAdvanced(args);
    const mode = base.mode ?? 'foreground';
    const steps = compileNewestWorkflow(goal, profile, mode);
    const hrefHint = goal.href_hint ?? profile.href_hint;
    const openArgs = {
      ...base,
      action: 'open',
      mode,
      url: profile.destination_url,
      ...(hrefHint ? { ready_when: { selector: `a[href*=${cssStringLiteral(hrefHint)}]` } } : {}),
      steps
    };
    if (!openArgs.idempotency_key) openArgs.idempotency_key = this.openKeys.keyFor(openArgs);
    const opened = await this.callTool('browser', openArgs);
    const openedValue = getStructured(opened);
    if (!openedValue?.session_id || !(openedValue?.tab_id ?? openedValue?.chrome_tab_id)) {
      return withStructuredPatch(opened, {
        goal,
        route: profile,
        orchestration: { external_tool_calls: 1, internal_tool_calls: 1, schema_discovery_calls: 0, repo_reads_required: 0, direct_route: true }
      });
    }
    const sessionId = openedValue.session_id;
    const tabId = openedValue.tab_id ?? openedValue.chrome_tab_id;
    const context = this.contexts.create(sessionId, tabId);
    const workflowValue = openedValue.workflow ?? {};
    const actualWorkflowCalls = Array.isArray(steps) ? 1 : 0;
    const verified = !failureLike(workflowValue) && hasPassedVerify(workflowValue);
    if (verified) this.routes.recordSuccess(goal, profile);
    else this.routes.recordFailure(goal, { structuralDrift: structuralDrift(workflowValue) });
    const status = verified ? 'verified' : (workflowValue.status ?? 'failed');
    const combined = {
      status,
      context,
      goal,
      route: {
        route_key: profile.route_key,
        source: profile.source,
        destination_url: profile.destination_url,
        confidence: profile.confidence,
        verification_required: true
      },
      workflow: workflowValue,
      orchestration: {
        external_tool_calls: 1,
        internal_tool_calls: actualWorkflowCalls,
        schema_discovery_calls: 0,
        repo_reads_required: 0,
        targeted_snapshots: 1,
        homepage_detours: 0,
        direct_route: true
      }
    };
    return { content: [{ type: 'text', text: JSON.stringify(combined) }], structuredContent: combined };
  }

  async handleToolCall(name, args) {
    if (!SIX_TOOLS.includes(name)) throw new Error(`unknown Controlla tool: ${name}`);
    if (name === 'browser' && args?.goal) return this.runGoal(args);
    if (name === 'browser' && args?.action === 'open') {
      const reused = await this.reuseExistingOpen(args);
      if (reused) return reused;
    }
    const prepared = this.prepareArgs(name, args);
    let result;
    try {
      result = await this.callTool(name, prepared);
    } catch (error) {
      if (name !== 'browser' && args?.context && recoverableContextError(error)) {
        await this.rebindContext(args.context);
        const rebound = this.prepareArgs(name, args);
        result = await this.callTool(name, rebound);
      } else {
        throw error;
      }
    }
    if (name === 'browser' && prepared.action === 'open') return this.rememberContext(result).result;
    if (name === 'browser' && ['release', 'release_shared'].includes(prepared.action)) {
      const sessionId = prepared.session_id;
      if (sessionId) this.contexts.removeBySession(sessionId);
    }
    const context = args?.context ?? (this.contexts.size() === 1 ? [...this.contexts.contexts.keys()][0] : null);
    return context ? withStructuredPatch(result, { context }) : result;
  }
}

module.exports = {
  AgentUxRuntime,
  BUILTIN_ROUTE_PROFILES,
  ContextRegistry,
  GOAL_SCHEMA,
  OpenKeyCache,
  RouteMemory,
  SIX_TOOLS,
  canonicalSite,
  compactTool,
  compactTools,
  compileNewestWorkflow,
  getStructured,
  publicHttpUrl,
  routeKey,
  withStructuredPatch
};
