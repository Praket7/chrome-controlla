import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { chromium } from 'playwright';

// Dispatch-time rebinding resolves a stale rich-editor reference by stable
// identity in the live document. The resolver is a JavaScript program embedded
// in the runtime, so the only faithful test is the real program against a real
// DOM. Read the shipped source instead of copying the program here.
const runtimeSource = readFileSync(
  new URL('../crates/controlla-runtime/src/mcp/v3_full.rs', import.meta.url),
  'utf8',
);
const selectorTemplate = (() => {
  const match = runtimeSource.match(/r#"\(\(\)=\>\{const role=__ROLE__[\s\S]*?\}\)\(\)"#/);
  assert.ok(match, 'runtime must still ship the editor rebind resolver');
  return match[0].replace(/^r#"/, '').replace(/"#$/, '');
})();
const countCall = 'return {count:pick.length,editable:editable.length}';
assert.ok(selectorTemplate.includes(countCall), 'resolver must still report candidate counts');

function resolver(role, name, tag) {
  return selectorTemplate
    .replace('__ROLE__', JSON.stringify(role))
    .replace('__NAME__', JSON.stringify(name))
    .replace('__TAG__', JSON.stringify(tag));
}
function firstMatchResolver(role, name, tag) {
  return resolver(role, name, tag).replace(countCall, 'return pick[0]');
}

let browser;
let page;
before(async () => {
  browser = await chromium.launch({
    headless: true,
    ...(process.env.CONTROLLA_CHROME ? { executablePath: process.env.CONTROLLA_CHROME } : { channel: 'chrome' }),
  });
  page = await browser.newPage();
});
after(async () => { await browser?.close(); });

// A Google-Docs-like surface: one virtualized textbox over a canvas page.
const DOCS_FIXTURE = `
  <div class="kix-page" style="width:600px;height:800px"></div>
  <div id="editor" role="textbox" contenteditable="true" aria-label="Document content"></div>
`;

async function resolveCount(html, role, name, tag) {
  await page.setContent(html);
  return page.evaluate(expression => new Function(`return ${expression}`)(), resolver(role, name, tag));
}

test('a replaced editor is rebound to the live node by tag and accessible name', async () => {
  await page.setContent(DOCS_FIXTURE);
  const counts = await page.evaluate(expression => new Function(`return ${expression}`)(),
    resolver('textbox', 'Document content', 'DIV'));
  assert.deepEqual(counts, { count: 1, editable: 1 });
  const rebound = await page.evaluate(expression => {
    // Replace the observed node exactly as a virtualized editor re-renders.
    const prior = document.querySelector('#editor');
    const next = prior.cloneNode(false);
    next.dataset.generation = '2';
    prior.replaceWith(next);
    const live = new Function(`return ${expression}`)();
    return { generation: live?.dataset?.generation ?? null, same: live === next };
  }, firstMatchResolver('textbox', 'Document content', 'DIV'));
  assert.deepEqual(rebound, { generation: '2', same: true });
});

test('two editors sharing an identity are refused rather than guessed', async () => {
  const counts = await resolveCount(`${DOCS_FIXTURE}
    <div role="textbox" contenteditable="true" aria-label="Document content"></div>`,
  'textbox', 'Document content', 'DIV');
  assert.equal(counts.count, 2, 'an ambiguous identity must be visible to the caller');
  assert.equal(counts.editable, 2);
});

test('a renamed editor is not rebound', async () => {
  const counts = await resolveCount('<div role="textbox" contenteditable="true" aria-label="Comment"></div>',
    'textbox', 'Document content', 'DIV');
  assert.deepEqual(counts, { count: 0, editable: 1 });
});

test('a role=textbox surface is a candidate even without contenteditable', async () => {
  // Canvas editors such as Google Docs expose an ARIA textbox over the canvas
  // instead of a contenteditable host; treating only isContentEditable as rich
  // would make the editor unresolvable.
  const counts = await resolveCount('<div role="textbox" aria-label="Document content"></div>',
    'textbox', 'Document content', 'DIV');
  assert.deepEqual(counts, { count: 1, editable: 1 });
});

test('read-only and disabled editors are excluded from the candidate set', async () => {
  const counts = await resolveCount(`<div role="textbox" contenteditable="true" aria-label="Document content" aria-readonly="true"></div>
    <div role="textbox" contenteditable="true" aria-label="Document content" aria-disabled="true"></div>`,
  'textbox', 'Document content', 'DIV');
  assert.deepEqual(counts, { count: 0, editable: 0 });
});

test('an accessible name cannot break out of the resolver literal', async () => {
  const hostile = 'Document content");alert(1);const x=("';
  await page.setContent(`<div id="first" role="textbox" contenteditable="true"></div>
    <div id="second" role="textbox" contenteditable="true" aria-label="Document content"></div>`);
  await page.evaluate(name => document.querySelector('#first').setAttribute('aria-label', name), hostile);
  const counts = await page.evaluate(expression => new Function(`return ${expression}`)(),
    resolver('textbox', hostile, 'DIV'));
  assert.equal(counts.count, 1, 'only the exact, escaped name matches');
});

test('a textarea editor resolves by the same identity rules', async () => {
  const counts = await resolveCount('<textarea aria-label="Document content"></textarea>',
    'textbox', 'Document content', 'TEXTAREA');
  assert.deepEqual(counts, { count: 1, editable: 1 });
});

// Google Docs hosts its editable element in a blank same-origin iframe and
// keeps an unrelated editable in the top document. A top-document-only search
// finds the wrong surface, or nothing at all, once the revision moves.
// The top document keeps an unrelated editable (Docs does), so the frame host
// must be found without treating that control as a candidate or an ambiguity.
function frameFixture(label) {
  return `
    <div class="kix-page" style="width:600px;height:800px"></div>
    <div role="textbox" contenteditable="true" aria-label="Document title"></div>
    <iframe id="frame"></iframe>
    <script>
      document.querySelector('#frame').contentDocument.body.innerHTML =
        '<div role="textbox" contenteditable="true" aria-label="${label}"></div>';
    </script>
  `;
}

test('an editor inside a blank same-origin frame is rebound by identity', async () => {
  await page.setContent(frameFixture('Document content'));
  const counts = await page.evaluate(expression => new Function(`return ${expression}`)(),
    resolver('textbox', 'Document content', 'DIV'));
  assert.deepEqual(counts, { count: 1, editable: 2 }, 'the frame host is the candidate, not the top-document editor');
});

test('a rebound frame editor is the live node, not a copy', async () => {
  await page.setContent(frameFixture('Document content'));
  const rebound = await page.evaluate(expression => {
    const live = new Function(`return ${expression}`)();
    return { inFrame: live?.ownerDocument !== document, tag: live?.tagName ?? null };
  }, firstMatchResolver('textbox', 'Document content', 'DIV'));
  assert.deepEqual(rebound, { inFrame: true, tag: 'DIV' });
});

test('a duplicated frame identity is refused unless focus proves the surface', async () => {
  await page.setContent(frameFixture('Document content'));
  const ambiguous = await page.evaluate(expression => {
    const duplicate = document.createElement('div');
    duplicate.setAttribute('role', 'textbox');
    duplicate.setAttribute('contenteditable', 'true');
    duplicate.setAttribute('aria-label', 'Document content');
    duplicate.style.cssText = 'width:600px;height:40px';
    document.body.append(duplicate);
    return new Function(`return ${expression}`)();
  }, resolver('textbox', 'Document content', 'DIV'));
  assert.equal(ambiguous.count, 2, 'two painted hosts must stay ambiguous');

  await page.setContent(frameFixture('Document content'));
  const focused = await page.evaluate(expression => {
    const hidden = document.createElement('div');
    hidden.setAttribute('role', 'textbox');
    hidden.setAttribute('contenteditable', 'true');
    hidden.setAttribute('aria-label', 'Document content');
    hidden.style.cssText = 'display:none';
    document.body.append(hidden);
    document.querySelector('#frame').contentDocument.querySelector('[contenteditable]').focus();
    return new Function(`return ${expression}`)();
  }, resolver('textbox', 'Document content', 'DIV'));
  assert.equal(focused.count, 1, 'focus resolves an identity whose other candidates are unpainted');
});

// The dispatch guard decides whether a rich editor holds the value the caller
// observed. A canvas editor keeps invisible line markers in an otherwise empty
// host, so "empty" must mean the same thing here as it does in a snapshot.
const guardSource = (() => {
  const match = runtimeSource.match(/const EDITOR_GUARD: &str = r#"([\s\S]*?)"#;/);
  assert.ok(match, 'runtime must still ship the editor dispatch guard');
  return match[1];
})();

async function runGuard(html, expected, options = {}) {
  await page.setContent(html);
  return page.evaluate(({ source, expected, normalize, requireFocus }) => {
    const element = document.querySelector('#target');
    const guard = new Function(`return (${source})`)();
    return guard.call({ nodes: [element] }, 0, expected, false, normalize, requireFocus);
  }, { source: guardSource, expected, normalize: options.normalize ?? false, requireFocus: options.requireFocus ?? false });
}

test('an empty canvas editor with line markers reads as empty', async () => {
  const result = await runGuard('<div id="target" role="textbox" contenteditable="true"><br></div>', '');
  assert.equal(result.ok, true, JSON.stringify(result));
});

test('whitespace-only editor content matches an empty caller expectation', async () => {
  const result = await runGuard('<div id="target" role="textbox" contenteditable="true"><br><br></div>', '\n\n');
  assert.equal(result.ok, true, JSON.stringify(result));
});

test('real editor text is never treated as empty', async () => {
  const result = await runGuard('<div id="target" role="textbox" contenteditable="true">Hello</div>', '');
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'stale_value');
  assert.equal(result.value, 'Hello');
});

test('an exact rich value still passes and a partial value still fails', async () => {
  assert.equal((await runGuard('<div id="target" role="textbox" contenteditable="true">Hello</div>', 'Hello')).ok, true);
  assert.equal((await runGuard('<div id="target" role="textbox" contenteditable="true">Hello world</div>', 'Hello')).reason, 'stale_value');
});

test('a plain textarea keeps exact value semantics', async () => {
  assert.equal((await runGuard('<textarea id="target">   </textarea>', '   ')).ok, true);
  assert.equal((await runGuard('<textarea id="target">   </textarea>', '')).reason, 'stale_value');
});

// The extension's shared transaction (used by human-keys typing) has its own
// guard; it must agree with the runtime guard about what an empty editor is.
const transactionGuardSource = (() => {
  const extensionSource = readFileSync(
    new URL('../extensions/chrome-controlla/lib/input.js', import.meta.url), 'utf8');
  const match = extensionSource.match(/export const TRANSACTION_GUARD = `([\s\S]*?)`;/);
  assert.ok(match, 'extension must still ship the transaction guard');
  // The guard lives in a template literal, so evaluate it to get the program
  // the extension actually runs (doubled backslashes become single ones).
  return new Function(`return \`${match[1]}\`;`)();
})();

async function runTransactionGuard(html, expectedValue) {
  await page.setContent(html);
  return page.evaluate(({ source, expectedValue }) => {
    const guard = new Function(`return (${source})`)();
    return guard.call({ nodes: [document.querySelector('#target')] }, 'exact_value', expectedValue, null, 'append', true, true, true, 0);
  }, { source: transactionGuardSource, expectedValue });
}

test('the extension transaction accepts an empty canvas editor', async () => {
  const result = await runTransactionGuard('<div id="target" role="textbox" contenteditable="true"><br><br></div>', '');
  assert.equal(result.ok, true, JSON.stringify(result));
  assert.equal(result.value, '', 'the transaction base value must be empty, not invisible line markers');
});

test('the extension transaction still refuses real content', async () => {
  const result = await runTransactionGuard('<div id="target" role="textbox" contenteditable="true">Hello</div>', '');
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'stale_value');
  assert.equal(result.value, 'Hello');
});

const surfaceProbeSource = (() => {
  const extensionSource = readFileSync(
    new URL('../extensions/chrome-controlla/lib/input.js', import.meta.url), 'utf8');
  const match = extensionSource.match(/export const SURFACE_PROBE = `([\s\S]*?)`;/);
  assert.ok(match, 'extension must still ship the surface probe');
  return new Function(`return \`${match[1]}\`;`)();
})();

test('an event-target frame editor is reported unreadable, not verified', async () => {
  await page.setContent('<div class="kix-page"></div><iframe id="frame"></iframe>');
  const probe = await page.evaluate(source => {
    const frame = document.querySelector('#frame');
    const editor = frame.contentDocument.createElement('div');
    editor.setAttribute('role', 'textbox');
    editor.setAttribute('contenteditable', 'true');
    frame.contentDocument.body.append(editor);
    const run = new Function(`return (${source})`)();
    return run.call({ nodes: [editor] }, 0);
  }, surfaceProbeSource);
  assert.equal(probe.rich, true);
  assert.equal(probe.event_target_frame, true);
  assert.equal(probe.readable, false, 'a blank-frame editing surface cannot prove typed text');
});

test('a normal contenteditable stays readable', async () => {
  await page.setContent('<div id="target" role="textbox" contenteditable="true"></div>');
  const probe = await page.evaluate(source => {
    const run = new Function(`return (${source})`)();
    return run.call({ nodes: [document.querySelector('#target')] }, 0);
  }, surfaceProbeSource);
  assert.equal(probe.readable, true);
});
