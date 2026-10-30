// Conservative per-application logical resource identity.
//
// Exact URL equality is the only identity Controlla may act on without
// judgement. Some apps, however, serve one durable document under many view
// URLs (Google Docs `?tab=t.0`, `?tab=t.123`, `?usp=sharing`). Reusing the tab
// that already holds that document is safe; stripping query parameters
// globally is not, so every rule here names its app and its identifier shape.
//
// A key is only a *candidate*: the caller must still prove the same document
// identity, an unleased tab, and no unknown mutation before navigating.

const RULES = Object.freeze([
  {
    app: "google_docs",
    // Document IDs are long opaque tokens; a 20-character floor rejects
    // look-alike paths while accepting every real document ID.
    pattern: /^https:\/\/docs\.google\.com\/document\/(?:u\/\d+\/)?d\/([A-Za-z0-9_-]{20,})(?:[/?#]|$)/,
    id: match => match[1],
    confidence: "high",
  },
  {
    app: "google_sheets",
    pattern: /^https:\/\/docs\.google\.com\/spreadsheets\/(?:u\/\d+\/)?d\/([A-Za-z0-9_-]{20,})(?:[/?#]|$)/,
    id: match => match[1],
    confidence: "high",
  },
  {
    app: "google_slides",
    pattern: /^https:\/\/docs\.google\.com\/presentation\/(?:u\/\d+\/)?d\/([A-Za-z0-9_-]{20,})(?:[/?#]|$)/,
    id: match => match[1],
    confidence: "high",
  },
  {
    app: "github_issue",
    pattern: /^https:\/\/github\.com\/([A-Za-z0-9-]{1,39})\/([A-Za-z0-9._-]{1,100})\/(?:issues|pull)\/(\d{1,10})(?:[/?#]|$)/,
    id: match => `${match[1].toLowerCase()}/${match[2].toLowerCase()}#${match[3]}`,
    confidence: "high",
  },
  {
    app: "github_repo",
    pattern: /^https:\/\/github\.com\/([A-Za-z0-9-]{1,39})\/([A-Za-z0-9._-]{1,100})(?:[/?#]|$)/,
    id: match => `${match[1].toLowerCase()}/${match[2].toLowerCase()}`,
    confidence: "medium",
    // Repository pages carry no durable per-view identity beyond the repo, so
    // only the repository landing page qualifies; a deep path may be a
    // distinct resource (a file, a settings page, a diff).
    wholePath: true,
  },
  {
    app: "gmail_thread",
    // Gmail routes threads in the hash: #inbox/<16 hex>. Only the inbox route
    // is named; search and label routes stay unmatched on purpose.
    pattern: /^https:\/\/mail\.google\.com\/mail\/u\/\d+\/(?:[^#]*)#(?:inbox|all|sent|starred)\/([0-9a-f]{16})(?:[/?#]|$)/,
    id: match => match[1],
    confidence: "medium",
  },
]);

export function logicalResourceKey(value) {
  if (typeof value !== "string" || !value) return null;
  let url;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" || url.username || url.password) return null;
  for (const rule of RULES) {
    const match = rule.pattern.exec(url.href);
    if (!match) continue;
    if (rule.wholePath && url.pathname.replace(/\/+$/, "").split("/").filter(Boolean).length !== 2) continue;
    const id = rule.id(match);
    if (!id) continue;
    return { app: rule.app, key: `${rule.app}:${id}`, confidence: rule.confidence };
  }
  return null;
}

export function sameLogicalResource(left, right) {
  const a = logicalResourceKey(left);
  const b = logicalResourceKey(right);
  return Boolean(a && b && a.key === b.key);
}

// A navigation that only changes the view of the same resource keeps the
// document alive in place. Whole-page routes (GitHub repo -> file) must not
// claim this, so they are excluded by `wholePath` above and by requiring both
// sides to resolve to the same key.
export function isSameResourceView(fromUrl, toUrl) {
  const from = logicalResourceKey(fromUrl);
  const to = logicalResourceKey(toUrl);
  if (!from || !to || from.key !== to.key) return false;
  if (from.app !== "google_docs" && from.app !== "google_sheets" && from.app !== "google_slides") return false;
  return true;
}
