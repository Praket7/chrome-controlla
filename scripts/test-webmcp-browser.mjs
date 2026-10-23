import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { chromium } from 'playwright';

const directory = new URL('../crates/controlla-runtime/src/shared_page/', import.meta.url);
const discovery = readFileSync(new URL('webmcp_discovery.js', directory), 'utf8');
const invocation = readFileSync(new URL('webmcp_invocation.js', directory), 'utf8');
let browser, server, base;
before(async () => {
  server = createServer((req, res) => res.end('<!doctype html><title>WebMCP fixture</title>'));
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  base = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true, args:['--enable-blink-features=WebMCP'], ...(process.env.CONTROLLA_CHROME ? { executablePath:process.env.CONTROLLA_CHROME } : { channel:'chrome' }) });
});
after(async () => { await browser?.close(); if (server) await new Promise(resolve => server.close(resolve)); });

test('native WebMCP JSON schemas discover and execute a real registered tool', async t => {
  const page = await browser.newPage();
  try {
    await page.goto(base);
    const capabilities = await page.evaluate(() => ({ register:typeof document.modelContext?.registerTool, discover:typeof document.modelContext?.getTools, execute:typeof document.modelContext?.executeTool }));
    if (Object.values(capabilities).some(value => value !== 'function')) {
      const result = await page.evaluate(source => new Function(`return (${source})`)()(), discovery);
      assert.equal(result.available, false);
      t.skip(`WebMCP unavailable in Chrome ${browser.version()}: ${JSON.stringify(capabilities)}`);
      return;
    }
    const result = await page.evaluate(async ({ discovery, invocation }) => {
      const schema = {type:'object',properties:{query:{type:'string'}},required:['query']};
      globalThis.fixtureCalls = 0;
      globalThis.fixtureInput = null;
      await document.modelContext.registerTool({name:'read_fixture',description:'Read synthetic fixture',inputSchema:schema,execute:async input => { fixtureCalls++; fixtureInput = input; return {query:input.query}; }});
      const catalog = await new Function(`return (${discovery})`)()();
      const call = new Function(`return (${invocation})`)();
      const drift = await call('read_fixture',{query:'denied'},{type:'object',properties:{}},1000);
      const callsBefore = fixtureCalls;
      const invoked = await call('read_fixture',{query:'observed'},schema,1000);
      return {catalog,drift,callsBefore,invoked,input:fixtureInput,callsAfter:fixtureCalls};
    }, {discovery,invocation});
    assert.equal(result.catalog.available, true);
    assert.equal(typeof result.catalog.tools[0].input_schema, 'object');
    assert.equal(result.drift.schema_changed, true);
    assert.equal(result.callsBefore, 0, 'schema drift must prevent invocation');
    assert.equal(result.invoked.available, true);
    assert.equal(result.invoked.schema_changed, undefined);
    assert.equal(result.callsAfter, 1);
    assert.deepEqual(result.input, {query:'observed'}, 'native invocation must receive the exact decoded arguments');
    t.diagnostic(`Native WebMCP tested in Chrome ${browser.version()}, local flag enabled`);
  } finally { await page.close(); }
});
