import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { chromium } from 'playwright';

const source = readFileSync(new URL('../crates/controlla-runtime/src/shared_page/snapshot.js', import.meta.url), 'utf8');
let browser;
before(async () => {
  browser = await chromium.launch({ headless: true, ...(process.env.CONTROLLA_CHROME ? { executablePath: process.env.CONTROLLA_CHROME } : { channel: 'chrome' }) });
});
after(async () => { await browser?.close(); });

async function collect(html, config = {}) {
  const page = await browser.newPage();
  try {
    await page.setContent(html);
    return await page.evaluate(({ source, config }) => {
      let geometryReads = 0;
      const original = Element.prototype.getBoundingClientRect;
      Element.prototype.getBoundingClientRect = function () { geometryReads++; return original.call(this); };
      const snapshot = new Function(`return (${source})`)()({selector:'body',token:'test',max_items:20,max_text_chars:0,max_bytes:8192,max_scan_nodes:1000,...config});
      return { items: snapshot.items, nodeCount: snapshot.nodes.length, geometryReads, result: snapshot.public };
    }, { source, config });
  } finally { await page.close(); }
}

test('collector reads geometry once per candidate without losing action bounds', async () => {
  const result = await collect('<button>Save</button><button>Cancel</button>');
  assert.equal(result.items.length, 2);
  assert.equal(result.geometryReads, 2);
  for (const item of result.items) assert.ok(item.bounds.width > 0);
});

test('small collection budget preserves partial coverage rather than inventing uniqueness', async () => {
  const result = await collect('<button>Save</button>'.repeat(40), { goal: { query:'Save', exhaustive:true }, max_bytes:4096 });
  assert.ok(result.items.length < 40);
  assert.equal(result.result.truncated, true);
  assert.ok(result.result.omitted_by_limit > 0);
});

test('role-targeted search avoids layout reads for excluded controls', async () => {
  const result = await collect('<button>Other</button>'.repeat(30) + '<a href="/target">Target</a>', {
    goal: { query: 'Target', roles: ['link'], exhaustive: true },
  });
  assert.deepEqual(result.items.map(item => item.name), ['Target']);
  assert.equal(result.geometryReads, 1);
  assert.equal(result.result.scope_complete, true);
});

test('ranked eviction leaves one candidate whose reference indexes the retained nodes', async () => {
  // The exact match is discovered last and evicts the lower-ranked earlier
  // candidates under a one-item goal budget. Its reference must be renumbered
  // with the retained nodes; otherwise a fresh reference is rejected as stale
  // because its raw index is no longer smaller than the returned count.
  const result = await collect(
    '<button>Target low priority</button><button>Unrelated control</button><button>Target</button>',
    { goal: { query: 'Target' }, max_items: 1, max_bytes: 20000 },
  );
  assert.equal(result.items.length, 1);
  assert.equal(result.items[0].name, 'Target');
  assert.equal(result.items[0].reference, 'test:0');
  assert.equal(result.result.items.length, 1);
  assert.equal(result.nodeCount, 1);
});

test('goal href filtering retains matching links and reports an incomplete bounded scan', async () => {
  const result = await collect('<a href="/other">Other</a><a href="/target">Target</a><button>Later</button>', {
    goal: { href_hint: '/target', exhaustive: true }, max_scan_nodes: 3,
  });
  assert.deepEqual(result.items.map(item => item.name), ['Target']);
  assert.equal(result.geometryReads, 1);
  assert.equal(result.result.scope_complete, false);
  assert.equal(result.result.search_budget_exhausted, true);
});

test('exhaustive role filtering retains duplicate matches in open shadow roots', async () => {
  const result = await collect('<a href="/one">Target</a><div id="host"></div><script>document.querySelector("#host").attachShadow({mode:"open"}).innerHTML = \'<a href="/two">Target</a>\';</script>', {
    goal: { query: 'Target', roles: ['link'], exhaustive: true },
  });
  assert.equal(result.items.filter(item => item.name === 'Target').length, 2);
  assert.equal(result.result.shadow_roots_seen, 1);
  assert.equal(result.result.scope_complete, true);
});
