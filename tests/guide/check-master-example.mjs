import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const guide = await readFile(path.join(root, 'docs/MASTER_GUIDE.md'), 'utf8');
const section = guide.split('### Read-only workflow example\n')[1];
assert.ok(section, 'read-only workflow example section is present');
const example = section.match(/```json\s*([\s\S]*?)\s*```/);
assert.ok(example, 'example contains a fenced JSON request');

const request = JSON.parse(example[1]);
assert.deepEqual(Object.keys(request).sort(), ['idempotency_key', 'session_id', 'steps', 'target_ref']);
assert.equal(typeof request.session_id, 'string');
assert.equal(typeof request.idempotency_key, 'string');
assert.deepEqual(Object.keys(request.target_ref).sort(), [
  'account_revision', 'browser_generation', 'browser_instance_id', 'capability_revision',
  'document_revision', 'frame_id', 'frame_revision', 'principal', 'session_id',
  'target_id', 'target_revision',
].sort());
assert.equal(request.steps.length, 2);
assert.deepEqual(request.steps[0], {
  kind: 'observe',
  spec: {
    selector: 'main',
    fields: { heading: 'h1', text: 'body' },
    max_items: 10,
    max_text_chars: 1000,
    max_bytes: 4096,
    cursor: null,
  },
});
assert.deepEqual(request.steps[1], { kind: 'checkpoint' });
console.log('Master-guide example is valid JSON and matches the documented read-only workflow request shape.');
