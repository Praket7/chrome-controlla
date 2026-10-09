const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const source = fs.readFileSync(path.join(__dirname, 'background.js'), 'utf8');
assert.match(source, /const MAX_BATCH_ACTIONS = 64;/);
assert.match(source, /async function dispatchBatch\(/);
assert.match(source, /message\.type === "command" \|\| message\.type === "batch"/);
assert.match(source, /request\.type === "command"[\s\S]*request\.type === "batch"/);
assert.match(source, /batch_execution: true/);
assert.match(source, /batch_deadline: true/);
assert.match(source, /request\.deadline_ms/);
assert.match(source, /performance\.now\(\) - startedAt >= request\.deadline_ms/);
assert.match(source, /host_round_trips: 1/);
assert.match(source, /await dispatchCommand\(/);
assert.match(source, /pairedTabDocumentIds\.get\(tabId\) !== expectedDocumentId/);
assert.match(source, /frame\.documentId !== expectedDocumentId/);
console.log('V3 near-page batch contract is present and document-guarded.');
