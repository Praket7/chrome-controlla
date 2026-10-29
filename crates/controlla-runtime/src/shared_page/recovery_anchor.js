function(index, mode) {
  // Resolve a dispatch point for a control whose own geometry refuses the
  // pointer path. `mode` is `ancestor` (nearest ancestor that accepts pointer
  // events) or `self` (the control's own box). Returns viewport-root
  // coordinates, matching the click guard's coordinate space, so the caller can
  // dispatch trusted CDP input without recomputing frame offsets.
  const arr = this.nodes || null;
  const e = arr ? arr[index] : this;
  const fail = reason => ({ok: false, reason});
  if (!e) return fail('stale_reference');
  const doc = e.ownerDocument;
  const view = doc && doc.defaultView;
  if (!doc || !view) return fail('detached');
  const pointerEvents = n => {
    try { return view.getComputedStyle(n).pointerEvents; } catch (_) { return 'auto'; }
  };
  // An ancestor is only a valid substitute when it plausibly owns the click
  // behavior. Without this a refused control would dispatch into `body` and
  // turn a clean refusal into an unrelated mutation.
  const plausible = n => {
    const tag = String(n.tagName).toUpperCase();
    if (typeof n.onclick === 'function' || n.hasAttribute('onclick')) return true;
    if (['BUTTON', 'A', 'LABEL', 'SUMMARY'].includes(tag)) return true;
    const role = n.getAttribute('role') || '';
    if (['button', 'link', 'checkbox', 'radio', 'menuitem', 'tab', 'option'].includes(role)) return true;
    return n.tabIndex >= 0 && n.tabIndex !== undefined && n !== document.body;
  };
  let anchor = e;
  if (mode === 'ancestor') {
    anchor = null;
    for (let n = e.parentElement, depth = 0; n && depth < 10; n = n.parentElement, depth++) {
      if (pointerEvents(n) !== 'none' && plausible(n)) { anchor = n; break; }
    }
    if (!anchor) return fail('unsupported_control');
  }
  anchor.scrollIntoView({block: 'center', inline: 'nearest'});
  const frame = doc === document ? null : (view.frameElement || null);
  if (frame) frame.scrollIntoView({block: 'center', inline: 'nearest'});
  const fb = frame ? frame.getBoundingClientRect() : null;
  const ox = fb ? fb.left : 0;
  const oy = fb ? fb.top : 0;
  const rect = anchor.getBoundingClientRect();
  if (!(rect.width > 0 && rect.height > 0)) return fail('hidden');
  const topView = document.defaultView || window;
  const hit = (x, y) => {
    let h = document.elementFromPoint(x + ox, y + oy);
    if (frame && h === frame) h = doc.elementFromPoint(x, y);
    for (let i = 0; h && h.shadowRoot && i < 8; i++) {
      const inner = h.shadowRoot.elementFromPoint(x + ox, y + oy);
      if (!inner || inner === h) break;
      h = inner;
    }
    return h;
  };
  const points = [[0.5, 0.5], [0.5, 0.3], [0.5, 0.7], [0.3, 0.5], [0.7, 0.5], [0.5, 0.12], [0.5, 0.88]];
  for (let i = 0; i < points.length; i++) {
    const x = rect.left + rect.width * points[i][0];
    const y = rect.top + rect.height * points[i][1];
    if (x < 0 || y < 0 || x >= topView.innerWidth || y >= topView.innerHeight) continue;
    const h = hit(x, y);
    if (!h) continue;
    // The control itself, or a descendant of the anchor, means the trusted
    // event will reach the same handler chain the refused control belongs to.
    if (h === anchor || anchor.contains(h) || h === e || e.contains(h)) {
      return {
        ok: true,
        x: x + ox,
        y: y + oy,
        via: mode,
        anchor: anchor === e ? null : String(anchor.tagName).toLowerCase(),
        viewport: JSON.stringify([topView.innerWidth, topView.innerHeight, topView.scrollX, topView.scrollY, topView.visualViewport?.width, topView.visualViewport?.height, topView.visualViewport?.offsetLeft, topView.visualViewport?.offsetTop, topView.visualViewport?.scale])
      };
    }
  }
  return fail('no_hit_target');
}
