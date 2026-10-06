import { createHash } from 'node:crypto';
import { access, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(await readFile(path.join(root, 'provenance/extraction.json'), 'utf8'));
const failures = [];

function containsExpectedTests(actual, expected) {
  return [...expected].every((test) => actual.has(test));
}

if (process.argv.includes('--self-test')) {
  const expected = new Set(['lib.rs:retained_test', 'session.rs:retained_test']);
  if (!containsExpectedTests(new Set(expected), expected)) throw new Error('retained-test positive control failed');
  if (containsExpectedTests(new Set(['lib.rs:retained_test']), expected)) throw new Error('retained-test missing-case control failed');
  console.log('Provenance retained-test self-check passed (present and missing controls).');
  process.exit(0);
}

async function hash(file) {
  return createHash('sha256').update(await readFile(file)).digest('hex');
}

for (const entry of manifest.files) {
  const destination = path.join(root, entry.destination_path);
  if (await hash(destination) !== entry.destination_sha256) failures.push(`destination hash: ${entry.destination_path}`);
  const source = path.resolve(root, entry.source_path);
  try {
    await access(source);
    if (await hash(source) !== entry.source_sha256) failures.push(`source hash: ${entry.source_path}`);
  } catch {
    // Source checkouts are optional; committed destination digests remain mandatory.
  }
}

const retained = [];
const testPattern = /#\[(?:tokio::)?test[^\]]*\]\s*(?:async\s+)?fn\s+(\w+)\s*\(/gs;
for (const destinationPath of new Set(manifest.retained_tests.cases.map((test) => test.destination_path))) {
  const source = await readFile(path.join(root, destinationPath), 'utf8');
  for (const match of source.matchAll(testPattern)) retained.push({ destination_path: destinationPath, test_name: match[1] });
}
const unique = new Map(retained.map((test) => [`${test.destination_path}:${test.test_name}`, test]));
const expected = new Set(manifest.retained_tests.cases.map((test) => `${test.destination_path}:${test.test_name}`));
if (unique.size < manifest.retained_tests.count || unique.size < expected.size || !containsExpectedTests(new Set(unique.keys()), expected)) {
  failures.push(`retained test inventory is incomplete: expected at least ${manifest.retained_tests.count} retained tests, found ${unique.size}`);
}

if (failures.length) {
  console.error(failures.join('\n'));
  process.exitCode = 1;
} else {
  console.log(`Verified ${manifest.files.length} destination digests and ${manifest.retained_tests.count} retained tests.`);
}
