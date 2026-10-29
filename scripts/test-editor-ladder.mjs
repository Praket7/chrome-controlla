import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { chromium } from 'playwright';

const runtime = new URL('../crates/controlla-runtime/src/', import.meta.url);
const editableSource = readFileSync(new URL('shared_page/recovery_editable.js', runtime), 'utf8');
const facade = readFileSync(new URL('mcp/v3_full.rs', runtime), 'utf8');
const start = facade.indexOf('const EDITOR_GUARD: &str = r#"');
assert.notEqual(start, -1, 'EDITOR_GUARD must stay a literal the test can load');
const bodyStart = start + 'const EDITOR_GUARD: &str = r#"'.length;
const bodyEnd = facade.indexOf('"#;', bodyStart);
assert.notEqual(bodyEnd, -1, 'EDITOR_GUARD literal must be terminated');
const editorGuard = facade.slice(bodyStart, bodyEnd);
// The guard maps a wrapper to `not_editable`; the ladder, not the guard, is
// what resolves the real editable. Loading the production literals keeps this
// test honest about the values that ship.
const wrapperPage = `<!doctype html><html><body>
<div id="shell" role="textbox" aria-label="Essay body">
  <div id="inner" contenteditable="true">seed</div>
</div>
</body></html>`;

let browser;
before(async () => {
  browser = await chromium.launch({
    headless: true,
    ...(process.env.CONTROLLA_CHROME ? { executablePath: process.env.CONTROLLA_CHROME } : { channel: 'chrome' }),
  });
});
after(async () => { await browser?.close(); });

async function evaluate(html, retained, fn) {
  const page = await browser.newPage();
  try {
    await page.setContent(html);
    return await page.evaluate(fn, { editorGuard, editableSource, retained });
  } finally {
    await page.close();
  }
}

test('a role=textbox shell is refused as not_editable instead of a focus failure', async () => {
  const result = await evaluate(wrapperPage, { nodes: [] }, ({ editorGuard, retained }) => {
    const shell = document.querySelector('#shell');
    retained.nodes[0] = shell;
    const guard = new Function(`return (${editorGuard})`)();
    return guard.call(retained, 0, 'seed', true, false, true, true);
  });
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'not_editable');
});

test('the editable rung rebinds the retained nodes array and returns the real editor', async () => {
  const result = await evaluate(wrapperPage, { nodes: [] }, ({ editableSource, retained }) => {
    const shell = document.querySelector('#shell');
    retained.nodes[0] = shell;
    const resolve = new Function(`return (${editableSource})`)();
    const target = resolve.call(retained, 0);
    return { id: target?.id ?? null, rebound: retained.nodes[0]?.id ?? null, same: target === document.querySelector('#inner') };
  });
  assert.deepEqual(result, { id: 'inner', rebound: 'inner', same: true });
});

test('the editable rung also resolves when the remote object is the shell itself', async () => {
  const result = await evaluate(wrapperPage, { nodes: null }, ({ editableSource }) => {
    const shell = document.querySelector('#shell');
    const resolve = new Function(`return (${editableSource})`)();
    const target = resolve.call(shell, 0);
    return { id: target?.id ?? null, same: target === document.querySelector('#inner') };
  });
  assert.deepEqual(result, { id: 'inner', same: true });
});

test('a role=textbox shell with no editable descendant resolves to nothing', async () => {
  const result = await evaluate('<div id="shell" role="textbox" aria-label="Locked">Locked</div>', { nodes: [] }, ({ editableSource, retained }) => {
    retained.nodes[0] = document.querySelector('#shell');
    const resolve = new Function(`return (${editableSource})`)();
    return { target: resolve.call(retained, 0) };
  });
  assert.equal(result.target, null);
});

test('after the rung the production guard accepts the editor and prepares a caret', async () => {
  const result = await evaluate(wrapperPage, { nodes: [] }, ({ editorGuard, editableSource, retained }) => {
    retained.nodes[0] = document.querySelector('#shell');
    const guard = new Function(`return (${editorGuard})`)();
    const resolve = new Function(`return (${editableSource})`)();
    resolve.call(retained, 0);
    const checked = guard.call(retained, 0, 'seed', true, false, true, true);
    return {
      ok: checked.ok,
      focused: document.activeElement?.id ?? null,
      text: document.querySelector('#inner').innerText,
    };
  });
  assert.equal(result.ok, true);
  assert.equal(result.focused, 'inner');
  assert.equal(result.text, 'seed');
});
