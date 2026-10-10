function(index, outcome, opts) {
  // Ordered, named predicates. Each failure names exactly one cause; never a generic "blocked".
  // `detail` is bounded safe metadata only: no text, values, passwords, or file paths.
  const RETRY = {outside_viewport:'safe_internal',obscured:'safe_internal',stale_reference:'refresh_snapshot',changed_target:'refresh_snapshot',detached:'refresh_snapshot',document_changed:'refresh_snapshot',hidden:'refresh_snapshot',no_hit_target:'refresh_snapshot',pointer_events_none:'refresh_snapshot',disabled:'refresh_snapshot',aria_disabled:'refresh_snapshot',masked_boundary:'requires_user',trusted_boundary:'requires_user',unsafe_page:'do_not_retry'};
  const fail = (reason, detail) => ({ok:false, reason, retry_class:RETRY[reason] || 'do_not_retry', detail:detail || {}});
  const round = n => Math.round(n);
  const rectOf = b => ({l:round(b.left), t:round(b.top), r:round(b.right), b:round(b.bottom)});
  const e = this.nodes[index], saved = this.items[index];
  const composedClosest = (node, selector) => {
    for (let n=node, depth=0; n && depth<16; n=n.parentElement || n.getRootNode()?.host, depth++) {
      if (n.matches?.(selector)) return n;
    }
    return null;
  };
  const align = (opts && opts.scroll) || 'nearest';
  if (!e || !saved) return fail('stale_reference');
  if (!e.isConnected) return fail('detached');
  const frame = e.ownerDocument === document ? null : e.ownerDocument.defaultView?.frameElement;
  if (e.ownerDocument !== document && (!frame || !frame.isConnected || frame.ownerDocument !== document)) return fail('document_changed');
  if (frame && e.tagName !== 'A') return fail('unsafe_page', {tag:e.tagName.toLowerCase()});
  if (this.raw(e) !== saved.raw_value || this.name(e) !== saved.name || (e.tagName === 'A' && e.href !== saved.href)) {
    return fail('changed_target', {field: this.raw(e) !== saved.raw_value ? 'value' : (this.name(e) !== saved.name ? 'name' : 'href')});
  }
  if (align !== 'none') e.scrollIntoView({block: align, inline: 'nearest'});
  if (frame && align !== 'none') frame.scrollIntoView({block:align, inline:'nearest'});
  const t = e.type || '', s = e.ownerDocument.defaultView.getComputedStyle(e), b = e.getBoundingClientRect();
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
    () => unsafe ? fail('unsafe_page', {tag: e.tagName.toLowerCase(), type: t || null}) : null,
    () => !this.visible(e) ? fail('hidden', {zero_size: !(b.width > 0 && b.height > 0), visibility: s.visibility, display: s.display, hidden_ancestor: !!e.closest('[hidden],[inert]')}) : null,
    () => e.matches(':disabled') ? fail('disabled') : null,
    () => e.getAttribute('aria-disabled') === 'true' ? fail('aria_disabled') : null,
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
  return {ok:true, x, y};
}
