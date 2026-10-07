import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const guide = await readFile(path.join(root, 'docs/MASTER_GUIDE.md'), 'utf8');
assert.match(guide, /no `jobs\.wait`/i, 'guide must not invent a polling tool absent from tools/list');
assert.match(guide, /supported `session` actions are `discover`, `targets`, `connect`, `list_targets`, `pair_shared`, `accept_shared`, `list_shared_targets`, and `release_shared`/);
assert.match(guide, /Shared tools use `chrome_tab_id`, not `target_ref`/);
assert.match(guide, /has no tab-close or generic session-release tool/);
assert.match(guide, /does not expose general `insert_text`, `key_sequence`, semantic click, or drag tools/);
assert.match(guide, /does not open or control a native OS file picker/);
assert.match(guide, /caller-supplied snapshot; it does not connect to Canva or independently verify session age\/page lock/);
assert.match(guide, /advances that element by one viewport/);
assert.match(guide, /independent of the traversed rows/);
assert.match(guide, /does not click a submit button or verify transfer, app acceptance, or persistence/);
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

const extractionSection = guide.split('### Extract and resume bounded results\n')[1];
assert.ok(extractionSection, 'extract cursor guidance is present');
const extractionExample = extractionSection.match(/```json\s*([\s\S]*?)\s*```/);
assert.ok(extractionExample, 'extraction example contains fenced JSON');
const extraction = JSON.parse(extractionExample[1]);
assert.deepEqual(Object.keys(extraction).sort(), ['max_bytes', 'max_records', 'sections', 'session_id', 'target_ref', 'timeout_ms'].sort());
assert.equal(extraction.sections.length, 1);
assert.equal(extraction.sections[0].spec.cursor, null);
assert.equal(extraction.sections[0].spec.id_field, 'data-id');
assert.ok(guide.includes('replace only `cursor` with that returned token'));
const fileSelectionSection = guide.split('### File selection request shape\n')[1];
assert.ok(fileSelectionSection, 'file selection schema guidance is present');
const fileSelectionExample = fileSelectionSection.match(/```json\s*([\s\S]*?)\s*```/);
assert.ok(fileSelectionExample, 'file selection example contains fenced JSON');
const fileSelection = JSON.parse(fileSelectionExample[1]);
assert.deepEqual(Object.keys(fileSelection).sort(), ['account_marker', 'artifact_handle', 'locator', 'session_id', 'target_ref'].sort());
assert.deepEqual(fileSelection.locator, { selector: "input[type='file']" });
assert.deepEqual(fileSelection.account_marker, ['[data-account]', 'test-account']);
console.log('Master-guide workflow and extraction examples match documented request shapes and supported tool names.');
