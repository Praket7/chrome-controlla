import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { resolveExtensionPackageAssets } from './extension-package-assets.mjs';

const root = resolve(fileURLToPath(new URL('..', import.meta.url)));

test('extension release assets include every manifest-referenced file exactly once', () => {
  const manifest = {
    background: { service_worker: 'background.js' },
    action: { default_popup: 'popup.html', default_icon: { 16: 'icons/16.png' } },
    options_page: 'options.html',
    content_scripts: [{ js: ['content.js'], css: ['content.css'] }],
    web_accessible_resources: [{ resources: ['assets/*.svg', 'recover.html'], matches: ['https://*/*'] }],
  };
  const files = resolveExtensionPackageAssets(manifest,
    ['background.js', 'popup.html', 'popup.js', 'secure-input.html', 'secure-input.js', 'secure-input.css', 'open-recovery.html', 'README.md', 'icons/16.png', 'options.html', 'content.js', 'content.css', 'recover.html', 'assets/icon.svg']);
  assert.ok(files.includes('popup.html'));
  assert.ok(files.includes('icons/16.png'));
  assert.ok(files.includes('assets/icon.svg'));
  assert.equal(new Set(files).size, files.length);
});

test('extension release asset validation rejects missing manifest resources', () => {
  assert.throws(() => resolveExtensionPackageAssets({ background: { service_worker: 'missing.js' } }, ['background.js']), /missing.js/);
});

test('extension release asset validation rejects omitted files matched by wildcard resources', () => {
  const manifest = { web_accessible_resources: [{ resources: ['assets/*.svg'], matches: ['https://*/*'] }] };
  assert.throws(() => resolveExtensionPackageAssets(manifest, ['assets/one.svg'], ['assets/one.svg', 'assets/two.svg']), /assets\/two\.svg .*assets\/\*\.svg/);
});

test('current Chrome extension build hash assets cover every manifest resource', async () => {
  const manifest = JSON.parse(await readFile(resolve(root, 'extensions/chrome-controlla/manifest.json'), 'utf8'));
  const background = await readFile(resolve(root, 'extensions/chrome-controlla/background.js'), 'utf8');
  const declaration = background.match(/const CONTROLLA_EXTENSION_BUILD_ASSETS = (\[[\s\S]*?\]);/);
  assert.ok(declaration, 'background must declare its build hash assets');
  const files = resolveExtensionPackageAssets(manifest, JSON.parse(declaration[1]));
  for (const asset of files) await readFile(resolve(root, 'extensions/chrome-controlla', asset));
});

test('release check runs the extension package asset contract', async () => {
  const packageJson = JSON.parse(await readFile(resolve(root, 'package.json'), 'utf8'));
  assert.match(packageJson.scripts['check:release'], /extension-package-assets\.test\.mjs/);
});
