import test from 'node:test';
import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { browserOutcomeElapsed, htmlReference, parseFinal, setup, summarizeToolEvents, toolOutcomeMatchesReference, verifiedCleanup, verifiedRead, verifyBrowserUseIdentity } from './phase11-public-readonly-pilot.mjs';

test('Controlla pilot uses one private task call without a launch round trip', () => {
  const prompt = setup('controlla', '/chrome', '/controlla', '/state').prompt;
  assert.match(prompt, /action":"task"/);
  assert.match(prompt, /mode":"headless"/);
  assert.match(prompt, /private task launches an isolated browser/);
  assert.doesNotMatch(prompt, /action":"launch"/);
  assert.match(prompt, /selector":"p:first-of-type"/);
  assert.doesNotMatch(prompt, /"action":"list"|"action":"launch"|"action":"snapshot"/);
  assert.match(prompt, /release_after_task/);
  assert.doesNotMatch(prompt, /action=release/);
});

test('DevTools pilot uses isolated headless local Chrome and requires page cleanup', () => {
  const setupResult = setup('chrome-devtools-mcp', '/chrome', '/controlla', '/state');
  assert.equal(setupResult.server, 'chrome-devtools');
  assert.match(setupResult.config.join(' '), /chrome-devtools-mcp@1\.10\.1/);
  assert.match(setupResult.config.join(' '), /--headless/);
  assert.match(setupResult.config.join(' '), /--isolated/);
  assert.match(setupResult.config.join(' '), /--executable-path/);
  assert.match(setupResult.prompt, /new page/);
  assert.match(setupResult.prompt, /close that page/);
});

test('agent-browser pilot uses a fresh non-restored headless Chrome session and cleanup', () => {
  const setupResult = setup('agent-browser', '/chrome', '/controlla', '/state');
  assert.equal(setupResult.server, 'agent-browser');
  assert.match(setupResult.config.join(' '), /agent-browser@0\.39\.1/);
  assert.match(setupResult.prompt, /headed=false/);
  assert.match(setupResult.prompt, /restore=false/);
  assert.match(setupResult.prompt, /restoreSave="never"/);
  assert.match(setupResult.prompt, /--engine","chrome","--executable-path/);
  assert.match(setupResult.prompt, /agent_browser_get_text/);
  assert.match(setupResult.prompt, /p:first-of-type/);
  assert.match(setupResult.prompt, /Copy the title from the successful open result/);
  assert.match(setupResult.prompt, /Return .* only if a required value is missing/);
  assert.match(setupResult.prompt, /close that same session once/);
  assert.match(setupResult.prompt, /exactly three browser tool calls/);
  assert.match(setupResult.prompt, /do not repeat or reopen/);
});

test('Browser Use pilot uses its installed MCP executable and isolates local headless Chrome state', () => {
  const configured = setup('browser-use', '/chrome', '/controlla', '/tmp/pilot-state', '/custom/browser-use');
  assert.equal(configured.server, 'browser-use');
  assert.match(configured.config.join(' '), /custom\/browser-use/);
  assert.doesNotMatch(configured.config.join(' '), /uvx|git\+https/);
  assert.match(configured.config.join(' '), /BROWSER_USE_HEADLESS/);
  assert.match(configured.config.join(' '), /BROWSER_USE_CONFIG_DIR/);
  assert.equal(configured.profileConfig.browser_profile.pilot.browser_binary_path, '/chrome');
  assert.equal(configured.profileConfig.browser_profile.pilot.user_data_dir, '/tmp/pilot-state/profile');
  assert.equal(configured.profileConfig.browser_profile.pilot.downloads_path, '/tmp/pilot-state/downloads');
  assert.equal(configured.profileConfig.browser_profile.pilot.headless, true);
  assert.match(configured.prompt, /exactly three calls/);
  assert.match(configured.prompt, /browser_get_html with no selector/);
  assert.match(configured.prompt, /browser_close_all once/);
  assert.match(configured.prompt, /Do not use retry_with_browser_use_agent/);
  const relative = setup('browser-use', '/chrome', '/controlla', '/state', './.venv/bin/browser-use');
  assert.ok(relative.config.join(' ').includes(resolve('./.venv/bin/browser-use')));
});

test('Browser Use identity rejects a different installed version or MCP source hash', () => {
  const expected = { version: '0.13.11', server_sha256: '696aee232f331ca215bb6e5ed21d5400dda1e4e530a6af6ade923e928acb2bd3' };
  assert.deepEqual(verifyBrowserUseIdentity(expected), expected);
  assert.throws(() => verifyBrowserUseIdentity({ ...expected, version: '0.14.0' }), /version mismatch/);
  assert.throws(() => verifyBrowserUseIdentity({ ...expected, server_sha256: 'different' }), /source hash mismatch/);
});

test('Browser Use outcome and cleanup require live HTML and one closed isolated session', () => {
  const reference = { title: 'Example Domain', paragraph: 'Current public fact.' };
  const event = (tool, text, elapsed_ms, arguments_ = {}) => ({ elapsed_ms, event: {
    type: 'item.completed', item: { type: 'mcp_tool_call', server: 'browser-use', tool, status: 'completed',
      arguments: arguments_, result: { content: [{ type: 'text', text }] } },
  } });
  const navigate = event('browser_navigate', 'Navigated to: https://example.com/', 10, { url: 'https://example.com/' });
  const read = event('browser_get_html', '<html><title>Example Domain</title><body><p>Current public fact.</p></body></html>', 20);
  const close = event('browser_close_all', 'Closed 1 sessions', 30);
  const events = [navigate, read, close];
  assert.equal(toolOutcomeMatchesReference(events, 'browser-use', 'browser-use', reference), true);
  assert.equal(browserOutcomeElapsed(events, 'browser-use', 'browser-use', reference), 20);
  assert.equal(verifiedCleanup(events, 'browser-use', 'browser-use'), true);
  assert.equal(toolOutcomeMatchesReference([navigate, close], 'browser-use', 'browser-use', reference), false);
  assert.equal(verifiedCleanup([navigate, read, event('browser_close_all', 'Closed 0 sessions', 30)], 'browser-use', 'browser-use'), false);
  assert.equal(verifiedCleanup([navigate, read, event('browser_close_all', 'Closed 1 sessions. Errors: failed', 30)], 'browser-use', 'browser-use'), false);
  assert.equal(verifiedCleanup([read, close], 'browser-use', 'browser-use'), false);
  assert.equal(verifiedCleanup([event('browser_navigate', 'Error: navigation failed', 10, { url: 'https://example.com/' }), read, close],
    'browser-use', 'browser-use'), false);
});

test('pilot retains bounded per-call receipts so transport completion cannot hide task failure', () => {
  const diagnostics = summarizeToolEvents([{
    elapsed_ms: 12,
    event: { type: 'item.completed', item: { type: 'mcp_tool_call', server: 'chrome-controlla', tool: 'browser', status: 'completed',
      arguments: { action: 'task' }, result: { structuredContent: { status: 'incomplete', reason_code: 'extract_failed',
        receipts: [{ kind: 'extract', status: 'not_verified', reason_code: 'extract_failed', snippet: 'selector did not match' }] } } } },
  }]);
  assert.equal(diagnostics[0].status, 'completed');
  assert.equal(diagnostics[0].arguments.action, 'task');
  assert.equal(diagnostics[0].result_summary.status, 'incomplete');
  assert.equal(diagnostics[0].result_summary.receipts[0].snippet, 'selector did not match');
});

test('pilot reference reads the actual public document rather than a model answer', () => {
  const reference = htmlReference('<html><head><title>Example Domain</title></head><body><p>Current public fact.</p></body></html>');
  assert.deepEqual(reference, { title: 'Example Domain', paragraph: 'Current public fact.' });
  assert.equal(parseFinal('{"title":"Example Domain","paragraph":"Current public fact."}')?.paragraph, 'Current public fact.');
  assert.equal(parseFinal('The title is Example Domain.'), null);
});

test('Playwright pilot supplies the required evaluate function and avoids a schema-error retry', () => {
  const prompt = setup('playwright-mcp', '/chrome', '/controlla', '/state').prompt;
  assert.match(prompt, /browser_evaluate.*function/);
  assert.match(prompt, /document\.title/);
  assert.match(prompt, /p:first-of-type/);
});

test('browser outcome is verified from successful tool results, independently of the final answer', () => {
  const reference = { title: 'Example Domain', paragraph: 'Current public fact.' };
  const events = [
    { event: { type: 'item.completed', item: { type: 'mcp_tool_call', server: 'agent-browser', tool: 'agent_browser_open', status: 'completed', result: { content: [{ type: 'text', text: JSON.stringify({ response: { data: { title: reference.title } } }) }] } } } },
    { event: { type: 'item.completed', item: { type: 'mcp_tool_call', server: 'agent-browser', tool: 'agent_browser_get_text', status: 'completed', result: { content: [{ type: 'text', text: JSON.stringify({ response: { data: { text: reference.paragraph } } }) }] } } } },
  ];
  assert.equal(toolOutcomeMatchesReference(events, 'agent-browser', 'agent-browser', reference), true);
  assert.equal(toolOutcomeMatchesReference(events.slice(0, 1), 'agent-browser', 'agent-browser', reference), false);
  assert.equal(toolOutcomeMatchesReference(events.map(({ event }) => ({ event: { ...event, item: { ...event.item, status: 'failed' } } })), 'agent-browser', 'agent-browser', reference), false);
});

test('pilot separates browser outcome timing from independent verifier timing', () => {
  const events = [
    { elapsed_ms: 120, event: { type: 'item.completed', item: { type: 'mcp_tool_call', server: 'chrome-controlla', tool: 'browser', status: 'completed', arguments: { action: 'task' } } } },
    { elapsed_ms: 150, event: { type: 'item.completed', item: { type: 'mcp_tool_call', server: 'chrome-controlla', tool: 'browser', status: 'completed', arguments: { action: 'release' } } } },
  ];
  assert.equal(browserOutcomeElapsed(events, 'controlla', 'chrome-controlla'), 120);
  assert.equal(browserOutcomeElapsed(events, 'playwright-mcp', 'playwright-mcp'), null);
});

test('browser outcome timing stops at the first successful exact tool evidence', () => {
  const reference = { title: 'Example Domain', paragraph: 'Current public fact.' };
  const event = (tool, elapsed_ms, value, status = 'completed') => ({ elapsed_ms, event: {
    type: 'item.completed', item: { type: 'mcp_tool_call', server: 'agent-browser', tool, status,
      result: { content: [{ type: 'text', text: JSON.stringify(value) }] } },
  } });
  const events = [
    event('agent_browser_open', 10, { response: { data: { title: reference.title } } }),
    event('agent_browser_get_text', 20, { response: { data: { text: reference.paragraph } } }),
    event('agent_browser_open', 30, { response: { data: { title: reference.title } } }),
    event('agent_browser_get_text', 40, { response: { data: { text: reference.paragraph } } }),
  ];
  assert.equal(browserOutcomeElapsed(events, 'agent-browser', 'agent-browser', reference), 20);
  assert.equal(browserOutcomeElapsed(events.slice(0, 1), 'agent-browser', 'agent-browser', reference), null);
});

test('cleanup verification requires an adapter-specific close postcondition', () => {
  const event = (server, tool, text, status = 'completed') => [{ event: {
    type: 'item.completed', item: { type: 'mcp_tool_call', server, tool, status, arguments: tool === 'browser' ? { action: 'task' } : undefined,
      result: { content: [{ type: 'text', text }] } },
  } }];
  assert.equal(verifiedCleanup(event('agent-browser', 'agent_browser_close', '{"response":{"data":{"closed":true}}}'), 'agent-browser', 'agent-browser'), true);
  assert.equal(verifiedCleanup([{ event: { type: 'item.completed', item: { type: 'mcp_tool_call', server: 'agent-browser', tool: 'agent_browser_close', status: 'completed', result: { response: { data: { closed: true } } } } } }], 'agent-browser', 'agent-browser'), true);
  assert.equal(verifiedCleanup(event('agent-browser', 'agent_browser_close', '{"response":{"data":{"closed":false}}}'), 'agent-browser', 'agent-browser'), false);
  assert.equal(verifiedCleanup(event('agent-browser', 'agent_browser_close', '{"message":"ok"}'), 'agent-browser', 'agent-browser'), false);
  assert.equal(verifiedCleanup(event('playwright-mcp', 'browser_close', '### Result\nNo open tabs. Navigate to a URL to create one.'), 'playwright-mcp', 'playwright-mcp'), true);
  assert.equal(verifiedCleanup(event('playwright-mcp', 'browser_close', 'close request accepted'), 'playwright-mcp', 'playwright-mcp'), false);
  const devtoolsOpen = { event: { type: 'item.completed', item: { type: 'mcp_tool_call', server: 'chrome-devtools', tool: 'new_page', status: 'completed',
    arguments: { url: 'https://example.com/' }, result: { content: [{ type: 'text', text: '## Pages\n1: about:blank\n2: Example Domain (https://example.com/) [selected]' }] } } } };
  const devtoolsClose = (pageId, text) => ({ event: { type: 'item.completed', item: { type: 'mcp_tool_call', server: 'chrome-devtools', tool: 'close_page', status: 'completed',
    arguments: { pageId }, result: { content: [{ type: 'text', text }] } } } });
  assert.equal(verifiedCleanup([devtoolsOpen, devtoolsClose(1, 'Note: the previously selected page was closed.\n## Pages\n1: about:blank [selected]')],
    'chrome-devtools-mcp', 'chrome-devtools', 'https://example.com/'), false, 'closing a different page must not qualify cleanup');
  assert.equal(verifiedCleanup([devtoolsOpen, devtoolsClose(2, 'Note: the previously selected page was closed.\n## Pages\n1: about:blank [selected]')],
    'chrome-devtools-mcp', 'chrome-devtools', 'https://example.com/'), true);
  assert.equal(verifiedCleanup([devtoolsClose(2, 'Note: the previously selected page was closed.\n## Pages\n1: about:blank [selected]'), devtoolsOpen],
    'chrome-devtools-mcp', 'chrome-devtools', 'https://example.com/'), false, 'a page opened after close cannot prove that close removed it');
  assert.equal(verifiedCleanup([devtoolsClose(2, 'Note: the previously selected page was closed.\n## Pages\n1: about:blank [selected]')],
    'chrome-devtools-mcp', 'chrome-devtools', 'https://example.com/'), false, 'cleanup must be tied to an observed target page');
  assert.equal(verifiedCleanup([devtoolsOpen, devtoolsClose(2, 'Note: the previously selected page was closed.\n## Pages\n2: Example Domain (https://example.com/) [selected]')],
    'chrome-devtools-mcp', 'chrome-devtools', 'https://example.com/'), false);
  assert.equal(verifiedCleanup(event('chrome-controlla', 'browser', '{"private_session_released":true}'), 'controlla', 'chrome-controlla'), true);
});

test('matching answer cannot qualify without an observed successful browser read', () => {
  const reference = { title: 'Example Domain', paragraph: 'Current public fact.' };
  const base = { exitCode: 0, error: null, verificationError: null, otherActionCount: 0,
    acted: true, observed: true, cleanupVerified: true, answer: { ...reference }, reference };
  assert.equal(verifiedRead(base), true);
  assert.equal(verifiedRead({ ...base, acted: false }), false);
  assert.equal(verifiedRead({ ...base, observed: false }), false);
  assert.equal(verifiedRead({ ...base, cleanupVerified: false }), false);
  assert.equal(verifiedRead({ ...base, otherActionCount: 1 }), false);
  assert.equal(verifiedRead({ ...base, answer: { ...reference, paragraph: 'stale content' } }), false);
  assert.equal(verifiedRead({ ...base, verificationError: 'source changed' }), false);
});
