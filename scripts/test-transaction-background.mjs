// The text transaction must keep working when the user leaves the tab.
//
// Chrome does not apply trusted key input to a tab it considers hidden: those
// events queue until the tab is focused again. The transaction therefore has to
// notice the change at a character boundary and finish the remaining text
// through the document. These tests emulate that behaviour (a capturing
// keydown handler swallows the key and the document reports itself hidden) so
// the switching logic is exercised without a background tab of a real window.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { typeTextTransaction } from "../extensions/chrome-controlla/lib/input.js";

let browser;
before(async () => {
  browser = await chromium.launch({
    headless: true,
    ...(process.env.CONTROLLA_CHROME ? { executablePath: process.env.CONTROLLA_CHROME } : { channel: "chrome" }),
  });
});
after(async () => { await browser?.close(); });

const FIXTURE = (RICH, CANVAS) => `<!doctype html><html><body style="margin:0">
${CANVAS
  ? '<div style="position:relative;width:660px;height:220px"><canvas width="640" height="200" style="position:absolute;left:0;top:0"></canvas><div id="ed" contenteditable="true" role="textbox" aria-label="Document content" style="position:absolute;left:16px;top:16px;width:600px;height:140px;font:16px monospace"></div></div>'
  : RICH
    ? '<div id="ed" contenteditable="true" role="textbox" style="width:640px;height:120px;border:1px solid #ccc"></div>'
    : '<textarea id="ed" style="width:640px;height:120px;font:16px monospace"></textarea>'}
<div id="blank" style="height:320px;border:1px dashed #999">blank</div>
<script>
window.__defer = false;
window.__hide = false;
const real = Object.getOwnPropertyDescriptor(Document.prototype, "visibilityState");
Object.defineProperty(Document.prototype, "visibilityState", {
  configurable: true,
  get() { return window.__hide ? "hidden" : real.get.call(this); },
});
document.addEventListener("keydown", event => {
  if (window.__defer) { event.preventDefault(); event.stopImmediatePropagation(); }
}, true);
</script></body></html>`;

async function run({ text, strategy = "key_events", pacing = 0, rich = false, canvas = false, hideAt = null, deferKeys = true, editAt = null, clickEmptyAt = null, replaceAt = null, restorePrefix = false, recoverPresentation = false }) {
  const page = await browser.newPage({ viewport: { width: 900, height: 700 } });
  try {
    await page.setContent(FIXTURE(rich, canvas));
    await page.bringToFront();
    await page.click("#ed");
    const cdp = await page.context().newCDPSession(page);
    const handle = await cdp.send("Runtime.evaluate", { expression: "document.getElementById('ed')" });
    let commands = 0;
    let presentationAcquisitions = 0;
    const ctx = {
      now: () => performance.now(),
      commandCount: () => commands,
      isAuthorized: () => true,
      sessionForFrame: () => null,
      loaderForFrame: () => null,
      syncFrameTree: async () => {},
      ensureAutoAttach: async () => {},
      waitForFrameSession: async () => null,
      sensitive: false,
      // A real session hands these to the extension's presentation slot. The
      // fixture only flips the emulated visibility flag: regaining the slot
      // means the window presents the tab again.
      acquirePresentation: async () => {
        presentationAcquisitions += 1;
        if (!recoverPresentation) return null;
        await page.evaluate(() => { window.__hide = false; window.__defer = false; });
        return "fixture-presentation-token";
      },
      revalidatePresentation: async () => true,
      releasePresentation: async () => {},
      async send(_target, method, params) { commands += 1; return cdp.send(method, params); },
    };
    const hide = () => page.evaluate(defer => { window.__defer = defer; window.__hide = true; }, deferKeys);
    if (hideAt === 0) await hide();
    const timers = [];
    if (hideAt > 0) timers.push(setTimeout(() => { hide().catch(() => {}); }, hideAt));
    if (replaceAt !== null) timers.push(setTimeout(() => {
      // A backgrounded tab can lose its renderer: Chrome swaps the document out
      // and restores the form afterwards, so every retained node is detached.
      page.evaluate(restore => {
        const old = document.getElementById("ed");
        const clone = old.cloneNode(true);
        clone.value = restore ? old.value : "";
        clone.textContent = restore ? old.textContent : "";
        old.replaceWith(clone);
      }, restorePrefix).catch(() => {});
    }, replaceAt));
    if (clickEmptyAt !== null) timers.push(setTimeout(() => {
      // A real click on the empty page area, the way a reader would take focus
      // away from the editor without touching its text.
      page.mouse.click(320, 260).catch(() => {});
    }, clickEmptyAt));
    if (editAt !== null) timers.push(setTimeout(() => {
      page.evaluate(() => {
        const ed = document.getElementById("ed");
        if (ed.value !== undefined) ed.value += " HUMAN";
        else ed.textContent += " HUMAN";
      }).catch(() => {});
    }, editAt));
    let receipt;
    try {
      receipt = await typeTextTransaction(1, {
        object_id: handle.result.objectId,
        index: 0,
        expected: { kind: "exact_value", value: "" },
        operation: "append",
        text,
        strategy,
        chunk_chars: 8,
        verify_every_chunks: 1,
        pacing_ms: pacing,
        normalize_paragraphs: true,
        deadline_ms: 60000,
      }, ctx);
    } finally {
      for (const timer of timers) clearTimeout(timer);
    }
    const value = await page.evaluate(() => {
      const ed = document.getElementById("ed");
      const raw = ed.value !== undefined ? ed.value : (ed.innerText ?? "");
      return raw.replace(/\n$/, "");
    });
    return { receipt, value, presentationAcquisitions };
  } finally {
    await page.close();
  }
}

const TEXT = "the quick brown fox jumps over the lazy dog";

test("a hidden document is typed through the document, not trusted keys", async () => {
  const { receipt, value } = await run({ text: TEXT, hideAt: 0, pacing: 5 });
  assert.equal(receipt.status, "verified");
  assert.equal(receipt.backend, "dom_typing");
  assert.equal(value, TEXT);
});

test("a transaction that loses the tab mid-way finishes the remaining text", async () => {
  const { receipt, value } = await run({ text: TEXT, hideAt: 120, pacing: 25 });
  assert.equal(receipt.status, "verified");
  assert.equal(receipt.backend, "dom_typing");
  assert.equal(receipt.interruption, null);
  assert.equal(value, TEXT);
});

test("a contenteditable keeps receiving text after the tab goes away", async () => {
  const { receipt, value } = await run({ text: TEXT, hideAt: 120, pacing: 25, rich: true });
  assert.equal(receipt.status, "verified");
  assert.equal(value, TEXT);
});

// A canvas editor's DOM holds nothing. Inserting the remaining text there used
// to look like success because the fixture reports its innerText, while a real
// canvas editor (Google Docs) would never receive it. The safe contract is: stop
// trusted input, report an explicit unknown, and never append a DOM tail.
test("a hidden canvas editor stops instead of faking success through DOM insertion", async () => {
  const { receipt, value } = await run({ text: TEXT, hideAt: 200, pacing: 25, canvas: true });
  assert.equal(receipt.status, "unknown", JSON.stringify(receipt));
  assert.equal(receipt.backend, "key_events", "the canvas transaction never switches to DOM insertion");
  assert.equal(receipt.reason, "canvas_presentation_lost");
  assert.ok(value.length > 0, "characters typed before the tab went hidden were delivered");
  assert.ok(value.length < TEXT.length, "the remaining text was not appended through the DOM");
  assert.ok(TEXT.startsWith(value), "what did land is an exact prefix of the requested text");
  assert.equal(receipt.completed_chars, value.length);
});

test("a canvas editor resumes after presentation is reacquired", async () => {
  const { receipt, value } = await run({ text: TEXT, hideAt: 200, pacing: 25, canvas: true, recoverPresentation: true });
  assert.equal(receipt.status, "verified", JSON.stringify(receipt));
  assert.equal(receipt.backend, "key_events");
  assert.equal(value, TEXT);
});

test("a detached editor is re-acquired and retyped from the new document", async () => {
  const { receipt, value } = await run({ text: TEXT, pacing: 25, replaceAt: 200 });
  assert.equal(receipt.status, "verified");
  assert.equal(receipt.interference_rebased, true);
  assert.equal(value, TEXT);
});

test("a restored prefix is resumed rather than retyped", async () => {
  const { receipt, value } = await run({ text: TEXT, pacing: 25, replaceAt: 200, restorePrefix: true });
  assert.equal(receipt.status, "verified");
  assert.equal(value, TEXT);
});

test("focus moving to empty page area does not end a paced transaction", async () => {
  const { receipt, value } = await run({ text: TEXT, pacing: 25, clickEmptyAt: 120 });
  assert.equal(receipt.status, "verified");
  assert.equal(receipt.interruption, null);
  assert.equal(value, TEXT);
});

test("a human edit still stops the transaction instead of overwriting it", async () => {
  const { receipt, value } = await run({ text: TEXT, pacing: 25, editAt: 120 });
  assert.notEqual(receipt.status, "verified");
  assert.ok(value.includes(" HUMAN"));
});

// The fast path is one bounded insertion per block, not per character. A plain
// control takes one Input.insertText; key_events stays zero.
test("bulk insertion types four thousand characters without per-character keys", async () => {
  const body = Array.from({ length: 80 }, (_, index) => `Line ${index}: the quick brown fox jumps over the lazy dog.\n`).join("").slice(0, 4000);
  const { receipt, value } = await run({ text: body, strategy: "bulk_insert", rich: true });
  assert.equal(receipt.status, "verified", JSON.stringify(receipt));
  assert.equal(receipt.bulk_insert, true);
  assert.equal(receipt.text_semantics, "plain_unicode_insert");
  assert.equal(receipt.key_events, 0);
  assert.ok(receipt.input_commands <= 2, `one insert command, got ${receipt.input_commands}`);
  assert.equal(value, body);
});

test("bulk insertion carries unicode verbatim", async () => {
  const text = "café 🙂 こんにちは\ncombining é and an em dash — done";
  const { receipt, value } = await run({ text, strategy: "bulk_insert", rich: true });
  assert.equal(receipt.status, "verified", JSON.stringify(receipt));
  assert.equal(receipt.text_semantics, "plain_unicode_insert");
  assert.equal(value, text);
});

// A canvas editor cannot be inserted into; it has to be presented first. The
// slot acquisition proves the fast path still refuses to fake delivery.
test("bulk insertion on a canvas editor takes the presentation slot first", async () => {
  const { receipt, value, presentationAcquisitions } = await run({ text: TEXT, strategy: "bulk_insert", canvas: true, hideAt: 0, recoverPresentation: true });
  assert.equal(receipt.status, "verified", JSON.stringify(receipt));
  assert.equal(receipt.bulk_insert, true);
  assert.ok(presentationAcquisitions >= 1, "the canvas was presented before dispatch");
  assert.ok(receipt.key_events < Array.from(TEXT).length, "a block, not a key per character");
  assert.equal(value, TEXT);
});

test("bulk insertion refuses a canvas editor it cannot present", async () => {
  const { receipt } = await run({ text: TEXT, strategy: "bulk_insert", canvas: true, hideAt: 0 });
  assert.equal(receipt.status, "not_dispatched", JSON.stringify(receipt));
  assert.equal(receipt.reason, "canvas_presentation_unavailable");
});
