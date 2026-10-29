import assert from "node:assert/strict";
import test from "node:test";

import { pruneSemanticItems, semanticDelta } from "./lib/observation.js";

const saveButton = (id, extra = {}) => ({
  role: "button",
  name: "Save",
  frame_id: "main",
  retained_reference: id,
  ...extra,
});

test("duplicate role/name controls keep distinct delta identities", () => {
  const previous = {
    items: [saveButton("node:1"), saveButton("node:2"), saveButton("node:3")],
  };
  const current = {
    items: [saveButton("node:1"), saveButton("node:2", { disabled: true }), saveButton("node:4")],
  };
  const delta = semanticDelta(previous, current, {
    documentId: "doc-1",
    previousDocumentId: "doc-1",
  });
  assert.equal(delta.added.length, 1);
  assert.equal(delta.added[0].retained_reference, "node:4");
  assert.equal(delta.changed.length, 1);
  assert.equal(delta.changed[0].retained_reference, "node:2");
  assert.equal(delta.removed.length, 1);
  assert.equal(delta.removed[0].retained_reference, "node:3");
  assert.equal(delta.unchanged, 1);
  assert.equal(delta.duplicate_keys, 0);
});

test("identical controls without stable identity are separated by occurrence", () => {
  const anonymous = { role: "button", name: "Save", frame_id: "main" };
  const previous = { items: [anonymous, anonymous] };
  const current = { items: [anonymous, anonymous] };
  const delta = semanticDelta(previous, current, {
    documentId: "doc-1",
    previousDocumentId: "doc-1",
  });
  assert.equal(delta.added.length, 0, "two anonymous duplicates must not register as new");
  assert.equal(delta.removed.length, 0);
  assert.equal(delta.unchanged, 2);
  assert.equal(delta.duplicate_keys, 0, "ordinal disambiguation prevents key collisions");
});

test("role and name collision alone never merges two live controls", () => {
  const previous = { items: [saveButton("node:1")] };
  const current = { items: [saveButton("node:1"), saveButton("node:2")] };
  const delta = semanticDelta(previous, current, {
    documentId: "doc-1",
    previousDocumentId: "doc-1",
  });
  assert.equal(delta.added.length, 1);
  assert.equal(delta.added[0].retained_reference, "node:2");
});

test("deltas stay document scoped and never carry references across a replacement", () => {
  const delta = semanticDelta(
    { items: [saveButton("node:1")] },
    { items: [saveButton("node:9")] },
    { documentId: "doc-2", previousDocumentId: "doc-1" },
  );
  assert.equal(delta.document_replaced, true);
  assert.deepEqual(delta.removed, []);
  assert.equal(delta.added.length, 1);
});

test("pruning reports measured retention instead of a constant floor", () => {
  const items = [
    { role: "textbox", name: "Email" },
    { role: "button", name: "Save" },
    { role: "button", name: "Cancel" },
  ];
  const pruned = pruneSemanticItems(items, {
    required_roles: ["textbox"],
    byte_budget: 40,
    max_items: 1,
  });
  assert.equal(pruned.retained_count, 1);
  assert.equal(pruned.required_total, 1);
  assert.equal(pruned.required_retained, 1);
  assert.equal(pruned.required_retention_rate, 1);
  assert.equal(pruned.dropped_required, 0);
  assert.equal(pruned.recall_floor, undefined, "the constant floor metric was removed");
});

test("pruning reports dropped required roles when the budget cannot hold them", () => {
  const items = [
    { role: "button", name: "Save", padding: "x".repeat(60) },
    { role: "button", name: "Cancel", padding: "y".repeat(60) },
  ];
  const pruned = pruneSemanticItems(items, {
    required_roles: ["button"],
    byte_budget: 80,
    max_items: 10,
  });
  assert.ok(pruned.required_total >= 2);
  assert.equal(pruned.required_retained, 1);
  assert.equal(pruned.required_retention_rate, 0.5);
  assert.equal(pruned.dropped_required, 1);
});
