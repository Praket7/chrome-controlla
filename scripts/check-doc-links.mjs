import { readdir, readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const files = [];

async function collect(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (entry.name === '.git' || entry.name === 'node_modules' || entry.name === 'target') continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) await collect(full);
    else if (entry.isFile() && entry.name.endsWith('.md')) files.push(full);
  }
}

await collect(root);
const broken = [];
for (const file of files) {
  const lines = (await readFile(file, 'utf8')).split(/\r?\n/);
  let fence = null;
  for (let index = 0; index < lines.length; index++) {
    const fenceMatch = lines[index].match(/^\s*(```+|~~~+)/);
    if (fenceMatch) {
      if (fence === null) fence = fenceMatch[1][0];
      else if (fence === fenceMatch[1][0]) fence = null;
      continue;
    }
    if (fence !== null) continue;
    for (const match of lines[index].matchAll(/\[[^\]]*\]\((<[^>]+>|[^\s)]+)(?:\s+[^)]*)?\)/g)) {
      const href = match[1].replace(/^<|>$/g, '');
      if (!href || /^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith('#') || href.startsWith('//')) continue;
      const targetPath = decodeURIComponent(href.split(/[?#]/, 1)[0]);
      const resolved = path.resolve(path.dirname(file), targetPath);
      try {
        await stat(resolved);
      } catch {
        broken.push(`${path.relative(root, file)}:${index + 1}: ${href}`);
      }
    }
  }
}

if (broken.length) {
  console.error(`Broken local Markdown links (${broken.length}):\n${broken.join('\n')}`);
  process.exitCode = 1;
} else {
  console.log(`Checked local Markdown links in ${files.length} files.`);
}
