// URL validation shared by navigation and open paths.

export function validateNavigationUrl(value) {
  if (typeof value !== "string" || value.length > 2048) throw new Error("URL must be a bounded HTTP(S) URL.");
  const url = new URL(value);
  if (!(url.protocol === "http:" || url.protocol === "https:") || !url.hostname || url.username || url.password) throw new Error("URL must be HTTP(S) and cannot contain credentials.");
  return url.href;
}

export function isSameDocumentUrl(previous, next) {
  try {
    const before = new URL(previous), after = new URL(next);
    return before.protocol === after.protocol && before.username === after.username && before.password === after.password
      && before.host === after.host && before.pathname === after.pathname && before.search === after.search
      && before.hash !== after.hash;
  } catch (_) { return false; }
}
