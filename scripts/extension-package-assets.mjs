const MANIFEST_PATH_FIELDS = [
  ['background', 'service_worker'],
  ['action', 'default_popup'],
  ['action', 'default_icon'],
  ['options_ui', 'page'],
  ['side_panel', 'default_path'],
];

function addPathValues(value, output) {
  if (typeof value === 'string') output.push(value);
  else if (value && typeof value === 'object' && !Array.isArray(value)) {
    for (const nested of Object.values(value)) addPathValues(nested, output);
  }
}

function manifestPaths(manifest) {
  const paths = [];
  for (const [parent, key] of MANIFEST_PATH_FIELDS) addPathValues(manifest?.[parent]?.[key], paths);
  for (const key of ['options_page', 'devtools_page']) addPathValues(manifest?.[key], paths);
  addPathValues(manifest?.chrome_url_overrides, paths);
  for (const script of manifest?.content_scripts ?? []) {
    addPathValues(script?.js, paths);
    addPathValues(script?.css, paths);
  }
  addPathValues(manifest?.icons, paths);
  for (const resourceSet of manifest?.web_accessible_resources ?? []) {
    for (const pattern of resourceSet?.resources ?? []) {
      if (typeof pattern === 'string') paths.push(pattern);
    }
  }
  return paths;
}

function matchesPattern(file, pattern) {
  const escaped = pattern.split('*').map(part => part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('.*');
  return new RegExp(`^${escaped}$`).test(file);
}

export function resolveExtensionPackageAssets(manifest, declaredAssets, availableAssets = declaredAssets) {
  if (!manifest || typeof manifest !== 'object' || !Array.isArray(declaredAssets)) {
    throw new TypeError('extension manifest and declared asset list are required');
  }
  if (!Array.isArray(availableAssets) || availableAssets.some(file => typeof file !== 'string')) {
    throw new TypeError('available extension asset list must be an array of file paths');
  }
  const assets = [...new Set([...declaredAssets, 'README.md'])];
  if (assets.some(file => typeof file !== 'string' || !file || file.startsWith('/') || file.split(/[\\/]/).includes('..'))) {
    throw new Error('extension package contains an invalid asset path');
  }
  const missing = manifestPaths(manifest).filter(pattern => !assets.some(file => matchesPattern(file, pattern)));
  if (missing.length) throw new Error(`extension package is missing manifest-referenced assets: ${[...new Set(missing)].join(', ')}`);
  const declared = new Set(assets);
  const omittedPatternMatches = manifestPaths(manifest)
    .filter(pattern => pattern.includes('*'))
    .flatMap(pattern => availableAssets.filter(file => matchesPattern(file, pattern) && !declared.has(file))
      .map(file => `${file} (matched by ${pattern})`));
  if (omittedPatternMatches.length) {
    throw new Error(`extension package omits files matched by manifest resource patterns: ${omittedPatternMatches.join(', ')}`);
  }
  return assets;
}
