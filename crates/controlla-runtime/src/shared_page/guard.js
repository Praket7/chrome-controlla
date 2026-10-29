function(index, outcome, opts) {
  // Ordered, named predicates. Each failure names exactly one cause; never a generic "blocked".
  // `detail` is bounded safe metadata only: no text, values, passwords, or file paths.
  // Mirrors the Rust execution-first table: a route/backend mismatch is
  // internal, only a real boundary (secret entry, a Chrome-owned surface, or a
  // fresh document) needs the caller. Keep this advisory table in sync with
  // `GuardReason::retry_class`.
  const RETRY = {outside_viewport:'safe_internal',obscured:'safe_internal',stale_reference:'safe_internal',changed_target:'safe_internal',detached:'safe_internal',document_changed:'refresh_snapshot',hidden:'refresh_snapshot',no_hit_target:'safe_internal',pointer_events_none:'safe_internal',disabled:'safe_internal',aria_disabled:'safe_internal',stale_value:'safe_internal',masked_boundary:'requires_user',trusted_boundary:'safe_internal',read_only:'safe_internal',unsupported_control:'safe_internal',unsupported_renderer_surface:'safe_internal',frame_local_route_required:'safe_internal',cross_origin_frame_unresolved:'safe_internal',protected_browser_surface:'do_not_retry',native_dialog_route_required:'safe_internal'};
  const fail = (reason, detail) => ({ok:false, reason, retry_class:RETRY[reason] || 'do_not_retry', detail:detail || {}});
  const round = n => Math.round(n);
  const rectOf = b => ({l:round(b.left), t:round(b.top), r:round(b.right), b:round(b.bottom)});
  let e = this.nodes[index];
  const saved = this.items[index];
  const t = e?.type || '';
  const composedClosest = (node, selector) => {
    for (let n=node, depth=0; n && depth<16; n=n.parentElement || n.getRootNode()?.host, depth++) {
      if (n.matches?.(selector)) return n;
    }
    return null;
  };
  const align = (opts && opts.scroll) || 'nearest';
  if (!e || !saved) return fail('stale_reference');
  if (!e.isConnected) {
    if (saved.source_frame !== 'main') return fail('detached');
    const defaultRole = n => n.getAttribute('role') || ({A:'link',BUTTON:'button',INPUT:'input',TEXTAREA:'textbox',SELECT:'combobox',SUMMARY:'button'}[n.tagName] || 'control');
    const candidates = Array.from(document.querySelectorAll('a[href],button,input,textarea,select,summary,[contenteditable]:not([contenteditable="false"]),[role]')).filter(n => {
      if (n.tagName.toLowerCase() !== saved.tag || defaultRole(n) !== saved.role || this.name(n) !== saved.name || this.raw(n) !== saved.raw_value || (n.tagName === 'A' && n.href !== saved.href)) return false;
      const ancestor = n.closest('article,li,[role="article"],[data-card],section');
      return !saved.semantic_ancestor?.id || ancestor?.getAttribute('data-id') === saved.semantic_ancestor.id || ancestor?.id === saved.semantic_ancestor.id;
    });
    if (candidates.length !== 1) return fail('detached', {reacquire_candidates:candidates.length});
    this.nodes[index] = e = candidates[0];
  }
  const frame = e.ownerDocument === document ? null : e.ownerDocument.defaultView?.frameElement;
  if (e.ownerDocument !== document && (!frame || !frame.isConnected || frame.ownerDocument !== document)) return fail('document_changed');
  // A child-document control is not inherently unsafe. The runtime chooses a
  // frame-local backend when one is available; only the actual unsafe-control
  // predicate below can produce `unsupported_renderer_surface`.
  if (this.raw(e) !== saved.raw_value || this.name(e) !== saved.name || (e.tagName === 'A' && e.href !== saved.href)) {
    return fail('changed_target', {field: this.raw(e) !== saved.raw_value ? 'value' : (this.name(e) !== saved.name ? 'name' : 'href')});
  }
  // Reject static safety boundaries before scrolling. A denied click must not
  // move the page and stale the caller's viewport-bound snapshot.
  if (composedClosest(e, '[data-masked]') || e.querySelector('[data-masked]')) return fail('masked_boundary');
  const trusted = composedClosest(e, '[data-requires-trusted]');
  const preScrollTrustedText = trusted === e && (e instanceof HTMLTextAreaElement || e instanceof HTMLInputElement && ['text','search','email','url','tel'].includes(e.type));
  if (e.querySelector('[data-requires-trusted]') || trusted && (trusted !== e || !preScrollTrustedText || outcome?.kind !== 'focused')) {
    return fail('trusted_boundary', {self: trusted === e});
  }
  if (__UNSAFE__) return fail('unsupported_renderer_surface', {tag:e.tagName.toLowerCase(), type:e.type || null});
  if (align !== 'none') e.scrollIntoView({block: align, inline: 'nearest'});
  if (frame && align !== 'none') frame.scrollIntoView({block:align, inline:'nearest'});
  const s = e.ownerDocument.defaultView.getComputedStyle(e), b = e.getBoundingClientRect();
  const fb = frame?.getBoundingClientRect();
  const x = b.left + b.width / 2 + (fb?.left || 0), y = b.top + b.height / 2 + (fb?.top || 0);
  const unsafe = __UNSAFE__;
  const trustedText = e.hasAttribute('data-requires-trusted') && (e instanceof HTMLTextAreaElement || e instanceof HTMLInputElement && ['text','search','email','url','tel'].includes(t));
  const trustedBoundary = composedClosest(e, '[data-requires-trusted]');
  const deepHit = () => {
    let h = document.elementFromPoint(x, y);
    if (frame && h === frame) h = e.ownerDocument.elementFromPoint(x - fb.left, y - fb.top);
    for (let i = 0; h && h.shadowRoot && i < 8; i++) { const inner = h.shadowRoot.elementFromPoint(x, y); if (!inner || inner === h) break; h = inner; }
    return h;
  };
  const protectedNode = n => !!composedClosest(n, '[data-masked],[data-requires-trusted],input[type="password"],input[type="file"],input[type="hidden"]');
  const ordered = [
    () => composedClosest(e, '[data-masked]') || e.querySelector('[data-masked]') ? fail('masked_boundary') : null,
    () => e.querySelector('[data-requires-trusted]') || trustedBoundary && (trustedBoundary !== e || !trustedText || outcome?.kind !== 'focused') ? fail('trusted_boundary', {self: trustedBoundary === e}) : null,
    () => unsafe ? fail('unsupported_renderer_surface', {tag: e.tagName.toLowerCase(), type: t || null}) : null,
    () => !this.visible(e) ? fail('hidden', {zero_size: !(b.width > 0 && b.height > 0), visibility: s.visibility, display: s.display, hidden_ancestor: !!e.closest('[hidden],[inert]')}) : null,
    () => e.matches(':disabled') ? fail('disabled') : null,
    // aria-disabled is an accessibility annotation, not a browser-level
    // pointer block. Keep native :disabled and the actual hit-test checks
    // authoritative here; a stale ARIA flag must not prevent a working
    // picker/button from being clicked. Fill/select guards still honor it.
    () => s.pointerEvents === 'none' ? fail('pointer_events_none') : null,
    () => (x < 0 || y < 0 || x >= innerWidth || y >= innerHeight) ? fail('outside_viewport', {rect: rectOf(b), viewport: {w: innerWidth, h: innerHeight}, scroll: align}) : null,
    () => !deepHit() ? fail('no_hit_target', {rect: rectOf(b)}) : null,
    () => {
      const h = deepHit();
      if (h === e || e.contains(h)) return null;
      // Never describe a protected blocker; report only that one exists.
      if (protectedNode(h)) return fail('obscured', {blocker: {protected: true}, rect: rectOf(b)});
      const hs = getComputedStyle(h), hb = h.getBoundingClientRect();
      const ix = Math.max(0, Math.min(b.right, hb.right) - Math.max(b.left, hb.left)), iy = Math.max(0, Math.min(b.bottom, hb.bottom) - Math.max(b.top, hb.top));
      return fail('obscured', {blocker: {tag: h.tagName.toLowerCase(), position: hs.position, z_index: hs.zIndex, role: h.getAttribute('role'), rect: rectOf(hb), covers_percent: b.width * b.height > 0 ? Math.min(100, round(ix * iy * 100 / (b.width * b.height))) : 0}, rect: rectOf(b)});
    },
  ];
  for (const check of ordered) { const r = check(); if (r) return r; }
  return {ok:true, x, y, viewport:JSON.stringify([innerWidth,innerHeight,scrollX,scrollY,visualViewport?.width,visualViewport?.height,visualViewport?.offsetLeft,visualViewport?.offsetTop,visualViewport?.scale])};
}
