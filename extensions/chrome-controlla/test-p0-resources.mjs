import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

import { configureOwnedTabOpener, reserveOwnedBackgroundSlot } from "./lib/owned-open.js";

import {
  PresentationSlotManager, PresentationRequirement, PresentationConflict,
  presentationProof, presentationSatisfies,
} from "./lib/presentation-slots.js";
import { logicalResourceKey, sameLogicalResource, isSameResourceView } from "./lib/resource-keys.js";
import {
  indexOwnedResources, selectReuse, usableRecord, openTelemetry, aliasReceipt,
  ReuseKind, TabDurability,
} from "./lib/owned-resources.js";

function clock(start = 1_000) {
  let value = start;
  return {
    now: () => value,
    advance: ms => { value += ms; },
    wait: async () => { value += 1; },
  };
}

test("one VISIBLE owner per window; a second task waits or is refused", async () => {
  const time = clock();
  const slots = new PresentationSlotManager({ now: time.now, wait: time.wait, pollMs: 1, staleMs: 10_000 });
  const first = await slots.reserve({ windowId: 7, tabId: 11, requirement: PresentationRequirement.VISIBLE, deadlineMs: 500 });
  assert.equal(first.granted, true);
  assert.equal(first.epoch, 1);

  const second = await slots.reserve({ windowId: 7, tabId: 12, requirement: PresentationRequirement.VISIBLE, deadlineMs: 2 });
  assert.equal(second.granted, false);
  assert.equal(second.reason, PresentationConflict.BUSY);
  assert.equal(second.owner.tab_id, 11, "the busy window reports the real owner instead of stealing");

  const released = await slots.release(first.token);
  assert.equal(released.released, true);
  const third = await slots.reserve({ windowId: 7, tabId: 12, requirement: PresentationRequirement.VISIBLE, deadlineMs: 0 });
  assert.equal(third.granted, true);
  assert.ok(third.epoch > first.epoch, "ownership change advances the presentation epoch");
});

test("a revalidated token is fenced by its epoch and can be rebound to a new tab", async () => {
  const time = clock();
  const slots = new PresentationSlotManager({ now: time.now, wait: time.wait, staleMs: 10_000 });
  const slot = await slots.reserve({ windowId: 3, tabId: null, requirement: PresentationRequirement.VISIBLE, deadlineMs: 0 });
  assert.equal((await slots.revalidate(slot.token)).valid, true);
  const bound = await slots.bind(slot.token, 44);
  assert.equal(bound.bound, true);
  assert.equal(slots.owner(3).tab_id, 44);

  await slots.releaseWindow(3, { reason: "test" });
  const stale = await slots.revalidate(slot.token);
  assert.equal(stale.valid, false, "a released token cannot keep dispatching input");
  assert.equal(stale.reason, "presentation_token_unknown");
});

test("an abandoned slot is reclaimed after the stale bound", async () => {
  const time = clock();
  const slots = new PresentationSlotManager({ now: time.now, wait: time.wait, staleMs: 500 });
  const first = await slots.reserve({ windowId: 9, tabId: 1, requirement: PresentationRequirement.VISIBLE, deadlineMs: 0 });
  assert.equal(first.granted, true);
  time.advance(600);
  const second = await slots.reserve({ windowId: 9, tabId: 2, requirement: PresentationRequirement.VISIBLE, deadlineMs: 0 });
  assert.equal(second.granted, true);
  assert.equal(second.reclaimed, true);
  assert.notEqual(second.token, first.token);
});

test("NONE never reserves and FOREGROUND requires focus", async () => {
  const slots = new PresentationSlotManager({ now: () => 1_000, wait: async () => {} });
  const none = await slots.reserve({ windowId: 4, requirement: PresentationRequirement.NONE });
  assert.equal(none.granted, true);
  assert.equal(none.token, null);
  assert.equal(slots.owner(4), null);

  const proof = presentationProof({ tab: { active: true }, window: { focused: false, state: "normal" }, renderer: { visibility_state: "visible" } });
  assert.equal(presentationSatisfies(proof, PresentationRequirement.VISIBLE).ok, true);
  assert.deepEqual(presentationSatisfies(proof, PresentationRequirement.FOREGROUND).missing, ["window_focused"]);
});

test("presentation proof rejects hidden, minimized, discarded and frozen surfaces", () => {
  const base = { tab: { active: true }, window: { focused: false, state: "normal" }, renderer: { visibility_state: "visible", document_has_focus: false } };
  const proof = presentationProof(base);
  assert.equal(presentationSatisfies(proof, PresentationRequirement.VISIBLE).ok, true);
  assert.deepEqual(presentationSatisfies({ ...proof, window_minimized: true }, PresentationRequirement.VISIBLE).missing, ["window_not_minimized"]);
  assert.deepEqual(presentationSatisfies({ ...proof, document_visibility: "hidden" }, PresentationRequirement.VISIBLE).missing, ["document_visible"]);
  assert.deepEqual(presentationSatisfies({ ...proof, tab_discarded: true }, PresentationRequirement.VISIBLE).missing, ["tab_not_discarded"]);
  assert.deepEqual(presentationSatisfies({ ...proof, tab_frozen: true }, PresentationRequirement.VISIBLE).missing, ["tab_not_frozen"]);
  assert.equal(presentationSatisfies(proof, PresentationRequirement.NONE).ok, true);
});

test("logical resource identity is app-specific, not a global query strip", () => {
  const docA = "https://docs.google.com/document/d/1AbCdEfGhIjKlMnOpQrStUvWxYz0123456789/edit?tab=t.0";
  const docAView = "https://docs.google.com/document/d/1AbCdEfGhIjKlMnOpQrStUvWxYz0123456789/edit?tab=t.123";
  const docB = "https://docs.google.com/document/d/1ZzYyXxWwVvUuTtSsRrQqPpOoNnMmLlKkJjIiHhGgFfEeDdCcBbAa/edit";
  assert.equal(logicalResourceKey(docA).key, logicalResourceKey(docAView).key);
  assert.equal(sameLogicalResource(docA, docAView), true);
  assert.equal(isSameResourceView(docA, docAView), true);
  assert.notEqual(logicalResourceKey(docA).key, logicalResourceKey(docB).key);

  assert.equal(logicalResourceKey("https://example.test/app?tab=t.0"), null, "unlisted apps keep exact-URL identity");

  const issue = logicalResourceKey("https://github.com/Praket7/chrome-controlla/issues/42");
  const pr = logicalResourceKey("https://github.com/Praket7/chrome-controlla/pull/42");
  const repo = logicalResourceKey("https://github.com/Praket7/chrome-controlla");
  const deep = logicalResourceKey("https://github.com/Praket7/chrome-controlla/blob/main/src/lib.rs");
  assert.equal(issue.key, "github_issue:praket7/chrome-controlla#42");
  assert.equal(pr.key, issue.key, "an issue and its pull request share one number space per repository");
  assert.equal(repo.key, "github_repo:praket7/chrome-controlla");
  assert.equal(deep, null, "a deep repository path is a distinct view, not the repository resource");

  assert.equal(logicalResourceKey("https://mail.google.com/mail/u/0/#inbox/17c2f9a8b1d3e4f5").key, "gmail_thread:17c2f9a8b1d3e4f5");
  assert.equal(logicalResourceKey("https://mail.google.com/mail/u/0/#search/anything"), null);
});

function receipt(overrides = {}) {
  return {
    status: "complete",
    tab_id: "12",
    window_id: 900,
    url: "https://example.test/a",
    document_id: "doc-a",
    create_background_window: true,
    created_ms: 1_000,
    last_used_ms: 1_000,
    last_verified_ms: 1_000,
    durability: TabDurability.EPHEMERAL,
    ...overrides,
  };
}

test("the owned-resource index aggregates aliases into one tab record", () => {
  const receipts = {
    "key-1": receipt(),
    "key-2": receipt({ created_ms: 2_000, last_used_ms: 2_000 }),
    "key-3": receipt({ tab_id: "13", url: "https://example.test/b", document_id: "doc-b", durability: TabDurability.SESSION }),
  };
  const index = indexOwnedResources(receipts);
  assert.equal(index.records.length, 2);
  const first = index.byTab.get(12);
  assert.deepEqual(first.keys.sort(), ["key-1", "key-2"]);
  assert.equal(first.created_ms, 2_000);
  assert.equal(first.durability, TabDurability.EPHEMERAL);
  assert.equal(index.byUrl.get("https://example.test/a").length, 1);
  assert.equal(index.byKey.get("key-3").tab_id, "13");
  assert.equal(usableRecord(first), true);
  assert.equal(usableRecord(first, { isLeased: () => true }), false, "a leased tab is never a reuse candidate");
});

test("reuse priority is exact document, then same resource, then an idle ephemeral tab", () => {
  const docA = "https://docs.google.com/document/d/1AbCdEfGhIjKlMnOpQrStUvWxYz0123456789/edit?tab=t.0";
  const receipts = {
    "docs-a": receipt({ tab_id: "21", window_id: 901, url: docA, document_id: "doc-21", durability: TabDurability.EPHEMERAL }),
    "other": receipt({ tab_id: "22", window_id: 901, url: "https://example.test/idle", document_id: "doc-22", last_used_ms: 0 }),
  };
  const index = indexOwnedResources(receipts);

  const exact = selectReuse(index, { url: docA, now: 5_000 });
  assert.equal(exact.kind, ReuseKind.EXACT_DOCUMENT);
  assert.equal(exact.record.tab_id, 21);

  const view = "https://docs.google.com/document/d/1AbCdEfGhIjKlMnOpQrStUvWxYz0123456789/edit?tab=t.99";
  const logical = selectReuse(index, { url: view, now: 5_000 });
  assert.equal(logical.kind, ReuseKind.SAME_RESOURCE_NAVIGATION);
  assert.equal(logical.navigate, true);

  const recycled = selectReuse(index, { url: "https://example.test/brand-new", now: 200_000, minIdleMs: 60_000 });
  assert.ok([ReuseKind.RECYCLED_OWNED_TAB, ReuseKind.SAME_RESOURCE_NAVIGATION].includes(recycled?.kind));
  assert.equal(recycled.record.tab_id, 22);

  const disabled = selectReuse(index, { url: "https://example.test/brand-new", now: 200_000, allowRecycle: false });
  assert.equal(disabled, null, "recycling is opt-in");
  const leasedIndex = indexOwnedResources(receipts);
  const allLeased = selectReuse(leasedIndex, { url: docA, now: 5_000, isLeased: () => true });
  assert.equal(allLeased, null, "a leased tab is never selected even for an exact document");
});

test("current-call telemetry distinguishes a created tab from a reused one", () => {
  const created = openTelemetry({ created: true, windowCreated: true });
  assert.deepEqual(created, {
    tab_created_this_call: true, tab_reused_this_call: false,
    window_created_this_call: true, window_reused_this_call: false,
    reuse_kind: ReuseKind.NEW_TAB_EXISTING_WINDOW,
  });
  const reused = openTelemetry({ created: false, windowCreated: false, reuseKind: ReuseKind.SAME_RESOURCE_NAVIGATION });
  assert.equal(reused.tab_created_this_call, false);
  assert.equal(reused.tab_reused_this_call, true);
  assert.equal(reused.reuse_kind, ReuseKind.SAME_RESOURCE_NAVIGATION);
});

test("an alias receipt resolves a new idempotency key to the existing tab", () => {
  const index = indexOwnedResources({ "ownerless:first-key": receipt() });
  const record = index.byTab.get(12);
  const alias = aliasReceipt(record, { key: "ownerless:second-key", url: "https://example.test/a" });
  assert.equal(alias.tab_id, "12");
  assert.equal(alias.document_id, "doc-a");
  assert.equal(alias.status, "complete");
  assert.equal(alias.alias_of, "ownerless:first-key");
  const after = indexOwnedResources({ "ownerless:first-key": receipt(), "ownerless:second-key": alias });
  assert.equal(after.records.length, 1, "the alias does not fork a second record for the same tab");
  assert.deepEqual(after.byTab.get(12).keys.sort(), ["ownerless:first-key", "ownerless:second-key"]);
});

// The harness concatenates lib modules into one script, which can hide a
// missing import: an unexported helper still resolves there but throws as real
// ESM. Import the lifecycle module directly so that illusion cannot survive.
test("the cold-open lifecycle is a real module with shared exports", () => {
  assert.equal(typeof reserveOwnedBackgroundSlot, "function");
  const configured = configureOwnedTabOpener({});
  assert.equal(typeof configured.openOwnedTabFromNative, "function");
  const background = fs.readFileSync(new URL("./background.js", import.meta.url), "utf8");
  assert.match(background, /import \{ configureOwnedTabOpener, reserveOwnedBackgroundSlot \} from "\.\/lib\/owned-open\.js";/,
    "background.js imports every lib helper it calls");
  assert.doesNotMatch(background, /async function reserveOwnedBackgroundSlot\(/,
    "the window-slot policy lives in the lib module, not background.js");
});

test("every declared extension build asset exists", () => {
  const background = fs.readFileSync(new URL("./background.js", import.meta.url), "utf8");
  const declaration = background.match(/const CONTROLLA_EXTENSION_BUILD_ASSETS = \[([^\]]+)\]/);
  assert.ok(declaration, "background.js declares its build asset set");
  const names = [...declaration[1].matchAll(/"([^"]+)"/g)].map(match => match[1]);
  assert.ok(names.length >= 18, `expected the full asset set, found ${names.length}`);
  for (const name of names) {
    assert.ok(fs.existsSync(new URL(`./${name}`, import.meta.url)), `declared build asset is missing: ${name}`);
  }
  assert.ok(names.includes("lib/owned-open.js"));
  assert.ok(names.includes("lib/presentation-slots.js"));
});

test("window presentations are per-window, not per-browser", async () => {
  const slots = new PresentationSlotManager({ now: () => 1_000, wait: async () => {} });
  const a = await slots.reserve({ windowId: 900, tabId: 1, requirement: PresentationRequirement.VISIBLE, deadlineMs: 0 });
  const b = await slots.reserve({ windowId: 901, tabId: 2, requirement: PresentationRequirement.VISIBLE, deadlineMs: 0 });
  assert.equal(a.granted && b.granted, true, "two windows can each present one tab");
  assert.equal(slots.owner(900).tab_id, 1);
  assert.equal(slots.owner(901).tab_id, 2);
});
