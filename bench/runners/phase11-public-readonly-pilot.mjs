#!/usr/bin/env node
// A deliberately narrow real-client pilot. Its output never satisfies the full
// Phase 11 release matrix or the broad superiority gate.
import { execFileSync, spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const targetUrl = 'https://example.com/';
const playwrightVersion = '0.0.83';
const devtoolsVersion = '1.10.1';
const agentBrowserVersion = '0.39.1';
const browserUseVersion = '0.13.11';
const browserUseCommit = 'c75e8476e26d18b7617643bc2ae082fae8eae431';
const browserUseServerSha256 = '696aee232f331ca215bb6e5ed21d5400dda1e4e530a6af6ade923e928acb2bd3';
const defaultBrowserUseExecutable = '/Users/pcg/.local/share/chrome-controlla-bench/browser-use-venv/bin/browser-use';
const stagehandCommit = 'cd7b230778cf92269e4cb90e80d97f5113781c51';
const stagehandFacadeSha256 = '12333b5a0362833ab5175c28dc021d5e9b6061d4c3bab6ca5855c3144b4e4692';
const defaultStagehandServer = '/Users/pcg/.local/share/chrome-controlla/comparison-mcps/stagehand-4.1.0/packages/integrations/core/dist/facade/stdio-server.mjs';
const adapters = ['controlla', 'playwright-mcp', 'chrome-devtools-mcp', 'agent-browser', 'browser-use', 'stagehand'];
const maxOutputBytes = 4_000_000;

function option(name, fallback) {
  const value = process.argv.find(arg => arg.startsWith(`--${name}=`));
  return value ? value.slice(name.length + 3) : fallback;
}

export function htmlReference(html) {
  const title = /<title\b[^>]*>([^<]*)<\/title>/i.exec(html)?.[1]?.trim();
  const paragraph = /<body\b[^>]*>[\s\S]*?<p\b[^>]*>([^<]*)<\/p>/i.exec(html)?.[1]?.trim();
  if (!title || !paragraph) throw new Error('public reference no longer has the expected title and paragraph');
  return { title, paragraph };
}

async function readReference() {
  const response = await fetch(targetUrl, { signal: AbortSignal.timeout(15_000), headers: { 'User-Agent': 'Controlla-Phase11-readonly-pilot/1.0' } });
  if (!response.ok || new URL(response.url).hostname !== 'example.com') throw new Error(`reference fetch failed or redirected: ${response.status} ${response.url}`);
  const html = await response.text();
  return { ...htmlReference(html), sha256: createHash('sha256').update(html).digest('hex') };
}

function toml(value) { return JSON.stringify(value); }

export function verifyBrowserUseIdentity(identity) {
  if (identity?.version !== browserUseVersion) throw new Error(`Browser Use version mismatch: expected ${browserUseVersion}, got ${identity?.version ?? 'unknown'}`);
  if (identity?.server_sha256 !== browserUseServerSha256) throw new Error(`Browser Use MCP source hash mismatch: expected ${browserUseServerSha256}, got ${identity?.server_sha256 ?? 'unknown'}`);
  return identity;
}

export function inspectBrowserUseExecutable(executable) {
  const firstLine = readFileSync(executable, 'utf8').split(/\r?\n/, 1)[0];
  const python = /^#!\s*(\S+)/.exec(firstLine)?.[1];
  if (!python) throw new Error(`Browser Use executable has no Python shebang: ${executable}`);
  const script = 'import hashlib, importlib.metadata, inspect, json, browser_use.mcp.server; p=inspect.getfile(browser_use.mcp.server); print(json.dumps({"version":importlib.metadata.version("browser-use"),"server_sha256":hashlib.sha256(open(p,"rb").read()).hexdigest()}))';
  const output = execFileSync(python, ['-c', script], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  return verifyBrowserUseIdentity(JSON.parse(output.trim()));
}

export function inspectStagehandServer(serverPath) {
  const absolutePath = resolve(serverPath);
  const repo = execFileSync('git', ['-C', dirname(absolutePath), 'rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
  const commit = execFileSync('git', ['-C', repo, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
  const tag = execFileSync('git', ['-C', repo, 'describe', '--tags', '--exact-match'], { encoding: 'utf8' }).trim();
  const dirty = execFileSync('git', ['-C', repo, 'status', '--porcelain'], { encoding: 'utf8' }).trim();
  const facade_sha256 = createHash('sha256').update(readFileSync(absolutePath)).digest('hex');
  if (commit !== stagehandCommit) throw new Error(`Stagehand source commit mismatch: expected ${stagehandCommit}, got ${commit}`);
  if (tag !== '@browserbasehq/stagehand@4.1.0') throw new Error(`Stagehand source tag mismatch: got ${tag}`);
  if (dirty) throw new Error('Stagehand source checkout has uncommitted changes');
  if (facade_sha256 !== stagehandFacadeSha256) throw new Error(`Stagehand facade hash mismatch: expected ${stagehandFacadeSha256}, got ${facade_sha256}`);
  return { commit, tag, facade_sha256, server_path: absolutePath, identity_scope: 'exact git commit/tag plus built stdio facade hash' };
}

function processInventory() {
  return execFileSync('ps', ['-Ao', 'pid=,command='], { encoding: 'utf8' }).split(/\r?\n/).filter(Boolean);
}

export function stagehandProcessCleanup(before, after, serverPath) {
  const normalizedServerPath = resolve(serverPath);
  if (after.some(line => line.includes(normalizedServerPath))) return false;
  const chromeProcesses = lines => lines.filter(line => /stagehand-chrome-|--user-data-dir=[^\s]*stagehand/i.test(line))
    .map(line => line.replace(/^\s*\d+\s+/, '').trim());
  const prior = new Map();
  for (const signature of chromeProcesses(before)) prior.set(signature, (prior.get(signature) ?? 0) + 1);
  for (const signature of chromeProcesses(after)) {
    const count = prior.get(signature) ?? 0;
    if (count === 0) return false;
    prior.set(signature, count - 1);
  }
  return true;
}

export function stagehandSingleCall(events, server) {
  const calls = events.filter(({ event }) => event.type === 'item.completed' && event.item?.type === 'mcp_tool_call'
    && event.item.server === server);
  return calls.length === 1 && calls[0].event.item.tool === 'run' && calls[0].event.item.status === 'completed'
    && !calls[0].event.item.error;
}

export function setup(adapter, browserPath, binaryPath, stateDir, browserUseExecutable = defaultBrowserUseExecutable,
  stagehandServerPath = defaultStagehandServer) {
  if (adapter === 'controlla') {
    return {
      server: 'chrome-controlla',
      config: [
        '-c', `mcp_servers.chrome-controlla.command=${toml(binaryPath)}`,
        '-c', `mcp_servers.chrome-controlla.args=${toml(['mcp'])}`,
        '-c', `mcp_servers.chrome-controlla.env={CONTROLLA_STATE_DIR = ${toml(stateDir)}, COMPTROL_CHROME_EXECUTABLE = ${toml(browserPath)}, CONTROLLA_AGENT_SURFACE = "v3-compact"}`,
      ],
      prompt: `Use only the chrome-controlla MCP browser tool. Make one call with {"action":"task","mode":"headless","release_after_task":true,"task":{"kind":"research","urls":["${targetUrl}"],"selector":"p:first-of-type","goal":"Read the document title and the first paragraph exactly as shown."}}. This one-call private task launches an isolated browser, reads and verifies the requested content, and releases the session; require private_session_released=true. Return exactly one JSON object with string fields "title" (document title) and "paragraph" (the first English paragraph in the page body), using the live task result only. Do not call launch, list, standalone snapshot, or separate release; do not use shell commands, web search, prior knowledge, or another browser tool. The final response must contain only that JSON object, with no introduction or Markdown. If the browser cannot be read, return {"error":"browser failed"}.`,
    };
  }
  if (adapter === 'playwright-mcp') return {
    server: 'playwright-mcp',
    config: [
      '-c', `mcp_servers.playwright-mcp.command=${toml('npx')}`,
      '-c', `mcp_servers.playwright-mcp.args=${toml(['-y', `@playwright/mcp@${playwrightVersion}`, '--browser', 'chrome', '--headless', '--isolated', '--executable-path', browserPath])}`,
    ],
    prompt: `Use only the playwright-mcp browser tools. Call browser_navigate with url ${targetUrl}. Then call browser_evaluate with the required function argument set to exactly "() => ({title: document.title, paragraph: document.querySelector('p:first-of-type')?.innerText.trim() || ''})". Copy both fields from that successful tool result exactly, then call browser_close once. Return exactly one JSON object with string fields "title" and "paragraph", with no introduction or Markdown. Do not call browser_evaluate without its function argument. Do not use shell commands, web search, prior knowledge, or another browser tool. If either value is missing or a browser call fails, return {"error":"browser failed"}.`,
  };
  if (adapter === 'chrome-devtools-mcp') return {
    server: 'chrome-devtools',
    config: [
      '-c', `mcp_servers.chrome-devtools.command=${toml('npx')}`,
      '-c', `mcp_servers.chrome-devtools.args=${toml(['-y', `chrome-devtools-mcp@${devtoolsVersion}`, '--headless', '--isolated', '--executable-path', browserPath, '--viewport', '1280x720'])}`,
    ],
    prompt: `Use only the Chrome DevTools MCP tools. Open ${targetUrl} in a new page, read the live page title and first paragraph with the page snapshot, and close that page. Return exactly one JSON object with string fields "title" and "paragraph", with no introduction or Markdown. Do not use shell commands, web search, prior knowledge, or another browser tool. If the browser cannot be read, return {"error":"browser failed"}.`,
  };
  if (adapter === 'agent-browser') return {
    server: 'agent-browser',
    config: [
      '-c', `mcp_servers.agent-browser.command=${toml('npx')}`,
      '-c', `mcp_servers.agent-browser.args=${toml(['-y', `agent-browser@${agentBrowserVersion}`, 'mcp'])}`,
    ],
    prompt: `Use only the agent-browser MCP tools. Make exactly three browser tool calls total: open ${targetUrl} once in a fresh uniquely named session with headed=false, restore=false, restoreSave="never", and extraArgs ["--engine","chrome","--executable-path",${JSON.stringify(browserPath)}]; call agent_browser_get_text once with that session and selector "p:first-of-type"; then close that same session once. Copy the title from the successful open result and the paragraph from get_text exactly. Once both reads succeed, do not repeat or reopen; the independent harness verifies the result. Return exactly one JSON object with fields "title" and "paragraph". Return {"error":"browser failed"} only if a required value is missing or one of those calls fails. No introduction or Markdown. Do not use shell commands, web search, prior knowledge, or another browser tool.`,
  };
  if (adapter === 'browser-use') return {
    server: 'browser-use',
    config: [
      '-c', `mcp_servers.browser-use.command=${toml(resolve(browserUseExecutable))}`,
      '-c', `mcp_servers.browser-use.args=${toml(['--mcp'])}`,
      '-c', `mcp_servers.browser-use.env={BROWSER_USE_HEADLESS = "true", BROWSER_USE_CONFIG_DIR = ${toml(stateDir)}, BROWSER_USE_DISABLE_EXTENSIONS = "true"}`,
    ],
    profileConfig: { browser_profile: { pilot: { id: 'pilot', default: true, headless: true,
      browser_binary_path: browserPath, user_data_dir: join(stateDir, 'profile'), downloads_path: join(stateDir, 'downloads'),
      keep_alive: true } }, llm: {}, agent: {} },
    prompt: `Use only the browser-use MCP direct browser tools. Make exactly three calls: browser_navigate with url ${targetUrl} and new_tab=false; browser_get_html with no selector to read the live document; then browser_close_all once to release the one isolated session. Read the document title and the first paragraph in the page body from the HTML result exactly. Do not use retry_with_browser_use_agent, browser_extract_content, shell commands, web search, prior knowledge, or any other browser tool. Return exactly one JSON object with string fields "title" and "paragraph", with no introduction or Markdown. Return {"error":"browser failed"} if the navigation or read fails or either value is missing.`,
  };
  if (adapter === 'stagehand') return {
    server: 'stagehand',
    config: [
      '-c', `mcp_servers.stagehand.command=${toml(process.execPath)}`,
      '-c', `mcp_servers.stagehand.args=${toml([resolve(stagehandServerPath)])}`,
      '-c', 'mcp_servers.stagehand.env={STAGEHAND_BROWSER = "local"}',
    ],
    prompt: `Use only the Stagehand MCP run tool. Make exactly one call with code that runs: await page.goto("${targetUrl}"); const url = page.url(); const title = await page.title(); const paragraph = (await page.locator("p:first-of-type").innerText()).trim(); await browser.close(); return JSON.stringify({title, paragraph, url, cleanup_requested: true}); Copy the values from the successful tool result exactly. Return exactly one JSON object with string fields "title" and "paragraph", with no introduction or Markdown. Do not use shell commands, web search, prior knowledge, another browser tool, or a second Stagehand call. Return {"error":"browser failed"} if any read or close request fails or a value is missing.`,
  };
  throw new Error(`unknown Phase 11 pilot adapter: ${adapter}`);
}

export function parseFinal(message) {
  try {
    const value = JSON.parse(message);
    if (typeof value?.title === 'string' && typeof value?.paragraph === 'string') return value;
  } catch { /* a non-JSON final answer is an unverified result */ }
  return null;
}

export function verifiedRead({ exitCode, error, verificationError, otherActionCount, acted, observed, cleanupVerified, answer, reference }) {
  return exitCode === 0 && !error && !verificationError && otherActionCount === 0 && acted && observed && cleanupVerified
    && answer?.title === reference?.title && answer?.paragraph === reference?.paragraph;
}

export function toolOutcomeMatchesReference(events, adapter, server, reference) {
  const expectedTool = adapter === 'controlla' ? 'browser'
    : adapter === 'playwright-mcp' ? 'browser_evaluate'
      : adapter === 'chrome-devtools-mcp' ? 'take_snapshot'
        : adapter === 'browser-use' ? 'browser_get_html' : 'agent_browser_open';
  const relevantTools = adapter === 'stagehand' ? new Set(['run'])
    : adapter === 'agent-browser' ? new Set(['agent_browser_open', 'agent_browser_get_text'])
    : new Set([expectedTool]);
  const successfulResults = events.filter(({ event }) => event.type === 'item.completed'
    && event.item?.type === 'mcp_tool_call' && event.item.server === server
    && event.item.status === 'completed' && !event.item.error
    && relevantTools.has(event.item.tool)
    && (adapter !== 'controlla' || event.item.arguments?.action === 'task'))
    .map(({ event }) => JSON.stringify(event.item.result ?? null));
  if (successfulResults.length === 0) return false;
  const evidence = successfulResults.join('\n');
  return evidence.includes(reference?.title ?? '\u0000') && evidence.includes(reference?.paragraph ?? '\u0000');
}

function decodeToolResult(result) {
  const structured = result?.structured_content ?? result?.structuredContent;
  if (structured && typeof structured === 'object') return { value: structured, text: JSON.stringify(structured) };
  if (result && typeof result === 'object' && !Array.isArray(result) && !Array.isArray(result.content)) {
    return { value: result, text: JSON.stringify(result) };
  }
  const text = result?.content?.find?.(entry => entry.type === 'text')?.text ?? '';
  try { return { value: JSON.parse(text), text }; } catch { return { value: null, text }; }
}

export function verifiedCleanup(events, adapter, server, targetUrl = 'https://example.com/', runtimeEvidence = {}) {
  const closeTool = adapter === 'controlla' ? 'browser'
    : adapter === 'playwright-mcp' ? 'browser_close'
      : adapter === 'chrome-devtools-mcp' ? 'close_page'
        : adapter === 'browser-use' ? 'browser_close_all' : adapter === 'stagehand' ? 'run' : 'agent_browser_close';
  let closeIndex = -1;
  for (let index = events.length - 1; index >= 0; index--) {
    const item = events[index].event?.item;
    if (events[index].event?.type === 'item.completed' && item?.type === 'mcp_tool_call' && item.server === server
        && item.tool === closeTool && item.status === 'completed' && !item.error
        && (adapter !== 'controlla' || item.arguments?.action === 'task')) {
      closeIndex = index;
      break;
    }
  }
  const closeEvent = closeIndex >= 0 ? events[closeIndex].event.item : null;
  if (!closeEvent) return false;
  const { value, text } = decodeToolResult(closeEvent.result);
  if (adapter === 'controlla') return value?.private_session_released === true;
  if (adapter === 'stagehand') return value?.cleanup_requested === true && runtimeEvidence.process_cleanup === true;
  if (adapter === 'agent-browser') return value?.response?.data?.closed === true || value?.data?.closed === true;
  if (adapter === 'playwright-mcp') return /No open tabs\b/i.test(text);
  if (adapter === 'browser-use') {
    const navigated = events.slice(0, closeIndex).some(({ event }) => event.type === 'item.completed'
      && event.item?.type === 'mcp_tool_call' && event.item.server === server && event.item.tool === 'browser_navigate'
      && event.item.status === 'completed' && !event.item.error && event.item.arguments?.url === targetUrl
      && decodeToolResult(event.item.result).text.includes(`Navigated to: ${targetUrl}`));
    return navigated && /^Closed 1 sessions\s*$/i.test(text.trim());
  }
  const openedTarget = events.slice(0, closeIndex).reverse().find(({ event }) => event.type === 'item.completed'
    && event.item?.type === 'mcp_tool_call' && event.item.server === server && event.item.tool === 'new_page'
    && event.item.status === 'completed' && !event.item.error && event.item.arguments?.url === targetUrl)?.event.item;
  if (!openedTarget || !/previously selected page was closed/i.test(text)) return false;
  const openedText = decodeToolResult(openedTarget.result).text;
  const targetPageLine = openedText.split(/\r?\n/).find(line => line.includes(targetUrl));
  const targetPageId = targetPageLine?.match(/^\s*(\d+):/)?.[1];
  const closedPageId = closeEvent.arguments?.pageId;
  const pagesInventory = text.match(/(?:^|\n)## Pages\s*\n([\s\S]*)/i)?.[1];
  const remainingPageIds = [...(pagesInventory ?? '').matchAll(/^\s*(\d+):/gm)].map(match => match[1]);
  return Boolean(targetPageId && closedPageId !== undefined && String(closedPageId) === targetPageId
    && pagesInventory !== undefined && !remainingPageIds.includes(targetPageId)
    && !pagesInventory.includes(targetUrl));
}

export function browserOutcomeElapsed(events, adapter, server, reference) {
  let evidence = '';
  for (const { event, elapsed_ms } of events) {
    if (event.type !== 'item.completed' || event.item?.type !== 'mcp_tool_call' || event.item.server !== server
        || event.item.status !== 'completed' || event.item.error) continue;
    const tool = event.item.tool;
    const relevant = adapter === 'controlla'
      ? tool === 'browser' && event.item.arguments?.action === 'task'
      : adapter === 'playwright-mcp' ? ['browser_snapshot', 'browser_evaluate'].includes(tool)
      : adapter === 'chrome-devtools-mcp' ? ['take_snapshot', 'evaluate_script'].includes(tool)
          : adapter === 'browser-use' ? tool === 'browser_get_html'
            : adapter === 'stagehand' ? tool === 'run'
              : ['agent_browser_open', 'agent_browser_get_text', 'agent_browser_get_title'].includes(tool);
    if (!relevant) continue;
    if (!reference) return elapsed_ms ?? null;
    evidence += JSON.stringify(event.item.result ?? null);
    if (evidence.includes(reference.title) && evidence.includes(reference.paragraph)) return elapsed_ms ?? null;
  }
  return null;
}

export function summarizeToolEvents(events) {
  return events.filter(({ event }) => event.type === 'item.completed' && event.item?.type === 'mcp_tool_call')
    .slice(0, 12).map(({ event, elapsed_ms }) => {
      const result = event.item.result;
      let payload = result?.structured_content ?? result?.structuredContent ?? null;
      if (!payload) {
        const text = result?.content?.find?.(entry => entry.type === 'text')?.text;
        if (text) { try { payload = JSON.parse(text); } catch { /* retain a bounded preview below */ } }
      }
      const receiptFields = ['kind', 'status', 'verified', 'reason_code', 'url', 'title', 'snippet', 'item_count', 'limited'];
      return {
        elapsed_ms,
        server: event.item.server,
        tool: event.item.tool,
        status: event.item.status,
        arguments: event.item.arguments ?? null,
        result_summary: payload && typeof payload === 'object' ? {
          status: payload.status,
          reason_code: payload.reason_code,
          title: payload.title,
          private_session_released: payload.private_session_released === true,
          url: payload.url,
          route: payload.execution_route?.tier,
          verification: payload.execution_route?.verification,
          metrics: payload.metrics && { elapsed_us: payload.metrics.elapsed_us,
            browser_commands_used: payload.metrics.browser_commands_used,
            host_round_trips: payload.metrics.host_round_trips },
          receipts: Array.isArray(payload.receipts) ? payload.receipts.map(receipt =>
            Object.fromEntries(receiptFields.filter(field => receipt[field] !== undefined).map(field => [field, receipt[field]]))) : undefined,
        } : null,
        result_preview: JSON.stringify(payload ?? result ?? event.item.error ?? null).slice(0, 4_000),
      };
    });
}

async function runAdapter(adapter, { codexPath, model, browserPath, binaryPath, browserUseExecutable, stagehandServerPath, timeoutMs, parentDir }) {
  const runDir = await mkdtemp(join(parentDir, `${adapter}-`));
  const stateDir = join(runDir, 'state');
  await mkdir(stateDir);
  const { server, config, prompt, profileConfig } = setup(adapter, browserPath, binaryPath, stateDir, browserUseExecutable, stagehandServerPath);
  if (profileConfig) await writeFile(join(stateDir, 'config.json'), `${JSON.stringify(profileConfig)}\n`);
  const promptHash = createHash('sha256').update(prompt).digest('hex');
  const args = ['exec', '--ephemeral', '--json', '--ignore-user-config', '--approve-for-me', '--skip-git-repo-check', '-m', model, '-C', runDir, ...config, prompt];
  const started = process.hrtime.bigint();
  const events = [];
  let finalAt = null;
  let stdout = '';
  let stderr = '';
  let buffer = '';
  let exitCode = null;
  let error = null;
  const stagehandProcessesBefore = adapter === 'stagehand' ? processInventory() : null;
  const consume = line => {
    if (!line.trim()) return;
    try {
      const event = JSON.parse(line);
      events.push({ event, elapsed_ms: Number(process.hrtime.bigint() - started) / 1_000_000 });
      if (event.type === 'item.completed' && event.item?.type === 'agent_message') finalAt = Number(process.hrtime.bigint() - started) / 1_000_000;
    } catch { /* preserve raw output hash without inventing an event */ }
  };
  try {
    await new Promise(resolveRun => {
      const child = spawn(codexPath, args, { cwd: runDir, env: process.env, stdio: ['ignore', 'pipe', 'pipe'] });
      let forceKill;
      const timer = setTimeout(() => {
        error = `deadline exceeded after ${timeoutMs} ms`;
        child.kill('SIGTERM');
        forceKill = setTimeout(() => child.kill('SIGKILL'), 2_000);
      }, timeoutMs);
      child.stdout.on('data', chunk => {
        if (Buffer.byteLength(stdout) >= maxOutputBytes) { error = 'stdout exceeded limit'; child.kill('SIGTERM'); return; }
        stdout += chunk;
        buffer += chunk;
        let end;
        while ((end = buffer.indexOf('\n')) >= 0) { consume(buffer.slice(0, end)); buffer = buffer.slice(end + 1); }
      });
      child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-16_000); });
      child.on('error', cause => { error = cause.message; clearTimeout(timer); clearTimeout(forceKill); resolveRun(); });
      child.on('close', code => { clearTimeout(timer); clearTimeout(forceKill); exitCode = code; consume(buffer); resolveRun(); });
    });
    const toolEvents = events.filter(({ event }) => event.type === 'item.completed' && event.item?.type === 'mcp_tool_call');
    const expectedTools = toolEvents.filter(({ event }) => event.item.server === server && event.item.status === 'completed' && !event.item.error);
    const otherActions = events.filter(({ event }) => event.type === 'item.completed' && ['command_execution', 'mcp_tool_call', 'web_search'].includes(event.item?.type)
      && !(event.item.type === 'mcp_tool_call' && event.item.server === server));
    const final = [...events].reverse().find(({ event }) => event.type === 'item.completed' && event.item?.type === 'agent_message')?.event.item.text ?? '';
    const answer = parseFinal(final);
    let reference = null;
    let verificationError = null;
    try { reference = await readReference(); } catch (cause) { verificationError = cause.message; }
    const verifiedAtMs = Number(process.hrtime.bigint() - started) / 1_000_000;
    const stagehandResult = adapter === 'stagehand'
      ? expectedTools.filter(({ event }) => event.item.tool === 'run').map(({ event }) => decodeToolResult(event.item.result).value).find(value => value && typeof value === 'object') ?? null
      : null;
    const acted = adapter === 'controlla'
      ? expectedTools.some(({ event }) => event.item.tool === 'browser' && event.item.arguments?.action === 'task'
        && event.item.arguments?.mode === 'headless' && !event.item.arguments?.session_id)
      : adapter === 'playwright-mcp'
        ? expectedTools.some(({ event }) => event.item.tool === 'browser_navigate' && event.item.arguments?.url === targetUrl)
        : adapter === 'chrome-devtools-mcp'
          ? expectedTools.some(({ event }) => ['new_page', 'navigate_page'].includes(event.item.tool) && event.item.arguments?.url === targetUrl)
          : adapter === 'browser-use'
            ? expectedTools.some(({ event }) => event.item.tool === 'browser_navigate' && event.item.arguments?.url === targetUrl
              && decodeToolResult(event.item.result).text.includes(`Navigated to: ${targetUrl}`))
            : adapter === 'stagehand'
              ? stagehandResult?.url === targetUrl
            : expectedTools.some(({ event }) => event.item.tool === 'agent_browser_open' && event.item.arguments?.url === targetUrl
            && event.item.arguments?.headed === false && event.item.arguments?.restore === false);
    const observed = expectedTools.some(({ event }) => adapter === 'controlla'
      ? event.item.tool === 'browser' && event.item.arguments?.action === 'task'
      : adapter === 'playwright-mcp' ? ['browser_snapshot', 'browser_evaluate'].includes(event.item.tool)
        : adapter === 'chrome-devtools-mcp' ? ['take_snapshot', 'evaluate_script'].includes(event.item.tool)
          : adapter === 'browser-use' ? event.item.tool === 'browser_get_html'
            : adapter === 'stagehand' ? event.item.tool === 'run' && typeof stagehandResult?.title === 'string' && typeof stagehandResult?.paragraph === 'string'
            : ['agent_browser_get_text', 'agent_browser_get_title'].includes(event.item.tool));
    const processCleanup = adapter === 'stagehand'
      ? stagehandProcessCleanup(stagehandProcessesBefore, processInventory(), stagehandServerPath)
      : undefined;
    const cleanupVerified = verifiedCleanup(events, adapter, server, targetUrl, { process_cleanup: processCleanup });
    const browserOutcomeVerified = toolOutcomeMatchesReference(events, adapter, server, reference);
    const finalAnswerMatchesReference = answer?.title === reference?.title && answer?.paragraph === reference?.paragraph;
    return {
      adapter, prompt_hash: promptHash,
      timing_origin: 'immediately_before_codex_cli_spawn',
      process_spawn_to_browser_outcome_ms: browserOutcomeElapsed(events, adapter, server, reference),
      process_spawn_to_independent_verification_ms: verifiedAtMs,
      process_spawn_to_final_ms: finalAt,
      first_browser_tool_started_ms: events.find(({ event }) => event.type === 'item.started' && event.item?.type === 'mcp_tool_call' && event.item.server === server)?.elapsed_ms ?? null,
      host_turns_completed: events.filter(({ event }) => event.type === 'turn.completed').length,
      model_round_trips: null,
      input_tokens: events.filter(({ event }) => event.type === 'turn.completed')
        .reduce((total, { event }) => total + (event.usage?.input_tokens ?? 0), 0),
      output_tokens: events.filter(({ event }) => event.type === 'turn.completed')
        .reduce((total, { event }) => total + (event.usage?.output_tokens ?? 0), 0),
      external_tool_calls: toolEvents.length, successful_browser_tool_calls: expectedTools.length,
      tool_names: toolEvents.map(({ event }) => `${event.item.server}.${event.item.tool}:${event.item.status}`),
      tool_diagnostics: summarizeToolEvents(events),
      failed_tool_errors: toolEvents.filter(({ event }) => event.item.status !== 'completed').map(({ event }) => ({
        tool: `${event.item.server}.${event.item.tool}`,
        error: JSON.stringify(event.item.error ?? event.item.result ?? null).slice(0, 1000),
      })),
      output_bytes: Buffer.byteLength(stdout), output_sha256: createHash('sha256').update(stdout).digest('hex'),
      final_answer_excerpt: final.slice(0, 1500),
      exit_code: exitCode, error: error ?? (exitCode === 0 ? null : stderr.trim() || `Codex exited ${exitCode}`),
      answer, reference_sha256: reference?.sha256 ?? null, verifier_error: verificationError,
      browser_outcome_matches_reference: browserOutcomeVerified,
      final_answer_matches_reference: finalAnswerMatchesReference,
      independently_verified_success: verifiedRead({ exitCode, error, verificationError,
        otherActionCount: otherActions.length, acted, observed, cleanupVerified, answer, reference })
        && browserOutcomeVerified && (adapter !== 'stagehand' || stagehandSingleCall(events, server)),
      evidence: { expected_server: server, expected_navigation_observed: acted, read_tool_observed: observed,
        cleanup_verified: cleanupVerified, browser_outcome_matches_reference: browserOutcomeVerified,
        final_answer_matches_reference: finalAnswerMatchesReference,
        cleanup_verification_method: cleanupVerified ? adapter === 'stagehand' ? 'browser_close_request_plus_os_process_inventory' : 'adapter_specific_close_result' : 'not_verified',
        ...(adapter === 'stagehand' ? { stagehand_process_cleanup_verified: processCleanup } : {}),
        ...(adapter === 'stagehand' ? { stagehand_single_call_contract_met: stagehandSingleCall(events, server) } : {}),
        unrelated_actions: otherActions.map(({ event }) => event.item?.type), reference: reference && { title: reference.title, paragraph: reference.paragraph } },
    };
  } finally {
    await rm(runDir, { recursive: true, force: true });
  }
}

async function main() {
  const repetitions = Number(option('repetitions', '1'));
  if (!Number.isInteger(repetitions) || repetitions < 1 || repetitions > 10) throw new Error('repetitions must be 1 to 10');
  const selectedAdapters = option('adapters', adapters.join(',')).split(',').map(value => value.trim()).filter(Boolean);
  if (!selectedAdapters.length || new Set(selectedAdapters).size !== selectedAdapters.length
      || selectedAdapters.some(adapter => !adapters.includes(adapter))) throw new Error('adapters must be a unique comma-separated subset of configured adapters');
  const codexPath = option('codex', '/Users/pcg/.local/bin/codex');
  const binaryPath = option('controlla', resolve(root, 'target/release/controlla-v2'));
  const browserPath = option('chrome', '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome');
  const browserUsePath = option('browser-use-executable', process.env.CONTROLLA_BROWSER_USE_EXECUTABLE ?? defaultBrowserUseExecutable);
  const browserUseIdentity = selectedAdapters.includes('browser-use') ? inspectBrowserUseExecutable(browserUsePath) : null;
  const stagehandServerPath = option('stagehand-server', process.env.CONTROLLA_STAGEHAND_SERVER ?? defaultStagehandServer);
  const stagehandIdentity = selectedAdapters.includes('stagehand') ? inspectStagehandServer(stagehandServerPath) : null;
  const model = option('model', 'gpt-6-luna');
  const timeoutMs = Number(option('deadline-ms', '120000'));
  const output = resolve(option('output', join(root, 'bench/reports', `phase11-public-readonly-pilot-${new Date().toISOString().replaceAll(/[:.]/g, '-')}.json`)));
  const chromeVersion = await new Promise((resolveVersion, rejectVersion) => {
    const child = spawn(browserPath, ['--version']); let data = '';
    child.stdout.on('data', chunk => { data += chunk; });
    child.on('error', rejectVersion);
    child.on('close', code => code === 0 ? resolveVersion(data.trim()) : rejectVersion(new Error(`Chrome version command exited ${code}`)));
  });
  const parentDir = await mkdtemp(join(tmpdir(), `controlla-phase11-pilot-${randomUUID()}-`));
  const report = { schema_version: 1, track: 'phase11-public-readonly-pilot', release_claim_eligible: false,
    scope: 'one public read-only fact task in isolated Chrome profiles; no signed-in state; no full Phase 11 matrix',
    task: { id: 'example-domain-title-paragraph', url: targetUrl, verifier: 'independent HTTPS fetch and exact title/paragraph match plus observed browser tool calls' },
    environment: { chrome_version: chromeVersion, model, selected_adapters: selectedAdapters,
      adapter_versions: { playwright_mcp: playwrightVersion, chrome_devtools_mcp: devtoolsVersion, agent_browser: agentBrowserVersion,
        ...(browserUseIdentity ? { browser_use: browserUseIdentity.version, browser_use_pinned_source_commit: browserUseCommit,
          browser_use_mcp_server_file_sha256: browserUseIdentity.server_sha256,
          browser_use_identity_scope: 'package version plus browser_use/mcp/server.py hash; full package Git commit not attested',
          browser_use_executable: resolve(browserUsePath) } : {}),
        ...(stagehandIdentity ? { stagehand_commit: stagehandIdentity.commit, stagehand_tag: stagehandIdentity.tag,
          stagehand_facade_sha256: stagehandIdentity.facade_sha256, stagehand_server_path: stagehandIdentity.server_path,
          stagehand_identity_scope: stagehandIdentity.identity_scope } : {}) },
      controlla_binary_sha256: createHash('sha256').update(await readFile(binaryPath)).digest('hex'),
      browser_path: browserPath, run_mode: 'cold_unknown', machine: process.platform + '-' + process.arch,
      timing_origin: 'immediately_before_codex_cli_spawn; Codex JSON events do not expose prompt-send dispatch' },
    pairs: [] };
  try {
    for (let i = 0; i < repetitions; i++) {
      const order = i % 2 === 0 ? selectedAdapters : [...selectedAdapters].reverse();
      const results = {};
      for (const adapter of order) results[adapter] = await runAdapter(adapter, { codexPath, model, browserPath, binaryPath,
        browserUseExecutable: browserUsePath, stagehandServerPath: stagehandIdentity?.server_path ?? stagehandServerPath, timeoutMs, parentDir });
      report.pairs.push({ replicate: i + 1, execution_order: order, ...results });
    }
    await mkdir(dirname(output), { recursive: true });
    await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
    console.log(output);
  } finally {
    await rm(parentDir, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.stack ?? error.message); process.exitCode = 1; });
}
