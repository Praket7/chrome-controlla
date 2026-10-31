// Generic rich-editor characterization. Adapters specialize preparation,
// readback, and persistence only; the model never selects an adapter. Generic
// strategies are tried before any application-specific behavior.

export const GENERIC_EDITOR_ADAPTER = Object.freeze({
  id: "generic",
  qualified: true,
  matches: () => true,
  prefers: "insert_text",
  requiresComposition: false,
});

export const GOOGLE_DOCS_ADAPTER = Object.freeze({
  id: "google_docs",
  qualified: false,
  matches: probe =>
    probe?.host === "docs.google.com" &&
    (probe?.hasEditorSurface === true || probe?.role === "textbox"),
  prefers: "insert_text",
  requiresComposition: false,
  readback: "editor_text",
  note: "Characterization only until an installed-Chrome qualification row passes.",
});

export const WORD_FOR_WEB_ADAPTER = Object.freeze({
  id: "word_for_web",
  qualified: false,
  matches: probe =>
    /(^|\.)office\.com$|(^|\.)microsoft365\.com$|(^|\.)live\.com$/i.test(probe?.host ?? "") &&
    probe?.hasEditorSurface === true,
  prefers: "key_events",
  requiresComposition: false,
  readback: "editor_text",
  note: "Characterization only until an installed-Chrome qualification row passes.",
});

export function describeEditorProbe(probe = {}) {
  const role = probe.role ?? null;
  const tag = probe.tag ?? null;
  const type = probe.type ?? null;
  const contenteditable = probe.contenteditable === true;
  const rich = contenteditable || (role === "textbox" && !["INPUT", "TEXTAREA"].includes(tag));
  const ordinary = tag === "TEXTAREA" || (tag === "INPUT" && ["text", "search", "email", "url", "tel"].includes(type));
  return {
    host: probe.host ?? null,
    role,
    tag,
    type,
    rich,
    ordinary,
    masked: probe.masked === true,
    readonly: probe.readonly === true,
    disabled: probe.disabled === true,
    requiresTrustedEvents: probe.requiresTrustedEvents === true,
    hasEditorSurface: probe.hasEditorSurface === true,
    selection: probe.selection ?? null,
    valueLength: Number.isSafeInteger(probe.valueLength) ? probe.valueLength : null,
  };
}

export function selectEditorAdapter(probe) {
  const described = describeEditorProbe(probe);
  for (const adapter of [GOOGLE_DOCS_ADAPTER, WORD_FOR_WEB_ADAPTER]) {
    if (adapter.qualified && adapter.matches(described)) return { adapter, described };
  }
  return { adapter: GENERIC_EDITOR_ADAPTER, described };
}

export function chooseInputBackend({ strategy, text, probe }) {
  const described = describeEditorProbe(probe);
  if (strategy === "native_setter") return { backend: "native_setter", reason: "explicit" };
  if (strategy === "insert_text") return { backend: "insert_text", reason: "explicit" };
  // `bulk_insert`, `auto`, `block`, and `insert_text` all insert the text as
  // written; the transaction decides whether that is one CDP insertion or a
  // bounded trusted key block for a canvas editor.
  if (strategy === "bulk_insert" || strategy === "block" || strategy === "auto") {
    return { backend: "insert_text", reason: "bulk_insert" };
  }
  if (strategy === "key_events") return { backend: "key_events", reason: "explicit" };
  if (strategy === "dom_typing") return { backend: "dom_typing", reason: "explicit" };
  if (strategy === "ime") return { backend: "ime_composition", reason: "explicit" };
  if (strategy === "clipboard_paste") return { backend: "clipboard_paste", reason: "explicit" };
  if (described.masked || described.requiresTrustedEvents) {
    return { backend: "key_events", reason: "trusted_surface" };
  }
  if (described.rich) return { backend: "insert_text", reason: "rich_editor_generic" };
  // Unicode is inserted as written. `stringNeedsComposition` is true only when
  // a caller asks for composition, not because a string has non-ASCII bytes.
  if (described.ordinary) return { backend: "insert_text", reason: "ordinary_text" };
  if (stringNeedsComposition(text)) return { backend: "ime_composition", reason: "composition_text" };
  return { backend: "insert_text", reason: "auto" };
}

export function stringNeedsComposition(text) {
  return typeof text === "string" && /[^\x00-\x7f]/u.test(text);
}

export function graphemeChunks(text, chunkChars) {
  const size = Math.max(1, Math.min(1024, Number.isSafeInteger(chunkChars) ? chunkChars : 64));
  const graphemes = [];
  if (typeof Intl !== "undefined" && typeof Intl.Segmenter === "function") {
    const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
    for (const part of segmenter.segment(text)) graphemes.push(part.segment);
  } else {
    graphemes.push(...Array.from(text));
  }
  const chunks = [];
  for (let index = 0; index < graphemes.length; index += size) {
    chunks.push(graphemes.slice(index, index + size).join(""));
  }
  return chunks;
}

export function codePointLength(text) {
  return Array.from(text ?? "").length;
}

export function hashText(text) {
  let hash = 0x811c9dc5;
  for (const byte of new TextEncoder().encode(text ?? "")) {
    hash ^= byte;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}
