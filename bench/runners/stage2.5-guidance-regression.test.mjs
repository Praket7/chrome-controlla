import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';

const read = path => fs.readFileSync(path, 'utf8');

test('root agent guidance forbids routine MCP and source discovery', () => {
  const agents = read('AGENTS.md');
  assert.match(agents, /do not list MCP servers/i);
  assert.match(agents, /do not .*inspect schemas/i);
  assert.match(agents, /do not .*browser describe/i);
  assert.match(agents, /do not .*MEMORY\.md/i);
  assert.match(agents, /zero repository reads/i);
  assert.doesNotMatch(agents, /client restart means reload schemas/i);
  assert.doesNotMatch(agents, /Navigate only to user-provided or observed URLs/i);
});

test('browser skill stays compact, compiled, and progressively discloses rare cases', () => {
  const path = 'packages/chrome-controlla/plugin/skills/browser-control/SKILL.md';
  const skill = read(path);
  assert.ok(Buffer.byteLength(skill) < 8_000, 'hot-path SKILL.md must stay under 8 KB');
  assert.match(skill, /Use them immediately/i);
  assert.match(skill, /do not list MCP servers/i);
  assert.match(skill, /references\/recovery\.md/);
  assert.match(skill, /references\/uploads-and-frames\.md/);
  assert.match(skill, /references\/advanced\.md/);
  assert.match(skill, /high-confidence synthesis/i);
  assert.match(skill, /Buffalo Bills story on ESPN/i);
  assert.match(skill, /"kind":"find_all"/);
  assert.match(skill, /"kind":"choose"/);
  assert.match(skill, /"reference":"\$latest"/);
  assert.doesNotMatch(skill, /Before the first call after an MCP reload, inspect the live `browser` schema/i);
});

test('plugin prompt tells the host to optimize full run rather than rediscover Controlla', () => {
  const plugin = JSON.parse(read('packages/chrome-controlla/plugin/plugin.json'));
  const prompt = plugin.extensions['com.openai'].interface.defaultPrompt;
  assert.match(prompt, /do not inspect MCP schemas/i);
  assert.match(prompt, /do not .*search repository source/i);
  assert.match(prompt, /prompt-to-verified-result latency/i);
});

test('maintained standalone client refreshes genuine schema drift internally', () => {
  const client = read('packages/chrome-controlla/bin/controlla-client.cjs');
  assert.match(client, /async function refreshSchemas\(\)/);
  assert.match(client, /notifications\/tools\/list_changed/);
  assert.match(client, /await refreshSchemas\(\)/);
  assert.match(client, /schema drift recovered automatically/i);
  assert.match(client, /caller's locally invalid arguments are not evidence of schema drift/i);
  assert.doesNotMatch(client, /Tool schemas changed; restart this client before further calls/);
});

test('Stage 2.5 has a persistent independently-qualified direct route memory', () => {
  const routes = read('crates/controlla-runtime/src/verified_route_memory.rs');
  assert.match(routes, /MissingQualificationRuns/);
  assert.match(routes, /Verification::Passed/);
  assert.match(routes, /verification_required: true/);
  assert.match(routes, /internal_model_calls: 0/);
  assert.match(routes, /structural drift/);
  assert.match(routes, /rolling route verification pass rate below 80 percent/);
  assert.match(routes, /token.*auth.*password/s);
});
