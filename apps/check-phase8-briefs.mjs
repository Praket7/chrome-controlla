import { readFile } from 'node:fs/promises';
import assert from 'node:assert/strict';

const files = [
  'apps/slides/acceptance.md',
  'apps/canva/acceptance.md',
  'apps/capcut-web/acceptance.md',
];
const required = [
  'pre-live brief only', 'Exact setup and initial state', 'Task steps',
  'Separate scoring rubrics', 'Evidence manifest (all pending)',
  'independent readback', 'persistence', 'export', 'Conflict and qualification cases',
  'foreground', 'strict background', 'dedicated headless',
];

for (const file of files) {
  const content = await readFile(new URL(`../${file}`, import.meta.url), 'utf8');
  for (const phrase of required) assert(content.toLowerCase().includes(phrase.toLowerCase()), `${file}: missing ${phrase}`);
  const manifest = content.split('## Evidence manifest (all pending)')[1]?.split('\n## ')[0] ?? '';
  assert(manifest.length > 0, `${file}: evidence manifest missing`);
  for (const row of manifest.split('\n').filter((line) => line.startsWith('|') && !line.includes('---'))) {
    if (row.includes('Required value')) continue;
    assert(row.trimEnd().endsWith('| pending |'), `${file}: non-pending evidence row: ${row}`);
  }
}
console.log(`Phase 8 pre-live briefs complete: ${files.length}`);
