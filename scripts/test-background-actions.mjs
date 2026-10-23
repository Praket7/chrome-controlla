import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { chromium } from 'playwright';

const source = readFileSync(new URL('../crates/controlla-runtime/src/shared_page/background_action.js', import.meta.url), 'utf8');
let browser;
before(async () => { browser = await chromium.launch({ headless: true, ...(process.env.CONTROLLA_CHROME ? { executablePath: process.env.CONTROLLA_CHROME } : { channel: 'chrome' }) }); });
after(async () => { await browser?.close(); });
async function run(html, action, expected, value, setup = '') {
  const page = await browser.newPage();
  try {
    await page.setContent(html);
    return await page.evaluate(({ source, action, expected, value, setup }) => {
      const retained = { nodes: [document.querySelector('#target')] };
      if (setup) new Function(setup)();
      const result = new Function(`return (${source})`)().call(retained, 0, action, expected, value, null);
      return { result, value: document.querySelector('#target')?.value, events: window.events || [] };
    }, { source, action, expected, value, setup });
  } finally { await page.close(); }
}
test('labelled fields verify exact value including whitespace', async () => {
  const actual = await run('<input id="target" aria-label="Name" value="old">', 'fill', 'old', ' new  value ');
  assert.equal(actual.result.verified, true);
  assert.equal(actual.result.after, ' new  value ');
});
test('stale expected value prevents overwrite', async () => {
  const actual = await run('<input id="target" value="human edit">', 'fill', 'old', 'new');
  assert.equal(actual.result.dispatched, false);
  assert.equal(actual.value, 'human edit');
  assert.equal(actual.result.reason, 'same_field_conflict');
});
test('replacement with identical name is not rebound', async () => {
  const actual = await run('<input id="target" aria-label="Name" value="old">', 'fill', 'old', 'new',
    'document.querySelector("#target").outerHTML = \'<input id="target" aria-label="Name" value="old">\'');
  assert.equal(actual.result.dispatched, false);
  assert.equal(actual.value, 'old');
});
test('live password transition refuses before reading a secret', async () => {
  const actual = await run('<input id="target" value="synthetic-secret">', 'fill', '', 'new',
    'document.querySelector("#target").type="password"');
  assert.equal(actual.result.dispatched, false);
  assert.equal(JSON.stringify(actual.result).includes('synthetic-secret'), false);
});
test('changed readback after events remains a dispatched mutation', async () => {
  const actual = await run('<input id="target" value="old">', 'fill', 'old', 'new',
    'document.querySelector("#target").addEventListener("input", e => e.target.value="server correction")');
  assert.equal(actual.result.dispatched, true);
  assert.equal(actual.result.verified, false);
  assert.equal(actual.value, 'server correction');
});
test('select verifies value rather than concatenated option labels', async () => {
  const actual = await run('<select id="target"><option value="a">First</option><option value="b">Second</option></select>', 'select', 'a', 'b');
  assert.equal(actual.result.verified, true);
  assert.equal(actual.result.after, 'b');
});
test('missing select option refuses without clearing selection', async () => {
  const actual = await run('<select id="target"><option value="a">First</option></select>', 'select', 'a', 'absent');
  assert.equal(actual.result.dispatched, false);
  assert.equal(actual.value, 'a');
});
test('double click delivers click click dblclick in order', async () => {
  const actual = await run('<button id="target">Open</button>', 'double_click', null, null,
    'window.events=[];for(const type of ["click","dblclick"])document.querySelector("#target").addEventListener(type,e=>window.events.push(e.type))');
  assert.deepEqual(actual.events, ['click', 'click', 'dblclick']);
  assert.equal(actual.result.dispatched, true);
});
test('sensitive transition during input hides post-dispatch value', async () => {
  const actual = await run('<input id="target" value="old">', 'fill', 'old', 'new',
    'document.querySelector("#target").addEventListener("input", e=>{e.target.type="password";e.target.value="synthetic-secret"})');
  assert.equal(actual.result.dispatched, true);
  assert.equal(JSON.stringify(actual.result).includes('synthetic-secret'), false);
});
