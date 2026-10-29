function(index) {
  // Rebind one semantic reference to the live node that carries the same
  // identity. The guard uses the same identity fields; this runs first so a
  // revalidation sees a connected target instead of `stale_reference`.
  const arr = this.nodes || null;
  if (!arr) return {ok: false, reason: 'stale_reference'};
  const saved = this.items && this.items[index];
  const current = arr[index];
  if (!saved) return {ok: false, reason: 'stale_reference'};
  if (current && current.isConnected) return {ok: true, rebound: false, candidates: 1};
  const tag = String(saved.tag || '').toUpperCase();
  const role = saved.role || '';
  const name = saved.name || '';
  const raw = saved.raw_value == null ? '' : saved.raw_value;
  const href = saved.href || '';
  const ancestorId = saved.semantic_ancestor && saved.semantic_ancestor.id;
  const defaultRole = n => n.getAttribute('role') ||
    ({A: 'link', BUTTON: 'button', INPUT: 'input', TEXTAREA: 'textbox', SELECT: 'combobox', SUMMARY: 'button'}[n.tagName] || 'control');
  const nameOf = n => (typeof this.name === 'function' ? this.name(n) : (n.getAttribute('aria-label') || n.textContent || ''));
  const rawOf = n => (typeof this.raw === 'function' ? this.raw(n) : ('value' in n ? n.value : (n.textContent || '')));
  const candidates = Array.from(document.querySelectorAll('a[href],button,input,textarea,select,summary,[contenteditable],[role]')).filter(n => {
    if (String(n.tagName).toUpperCase() !== tag) return false;
    if (defaultRole(n) !== role) return false;
    if (nameOf(n) !== name || rawOf(n) !== raw) return false;
    if (n.tagName === 'A' && n.href !== href) return false;
    if (ancestorId) {
      const ancestor = n.closest('article,li,[role="article"],[data-card],section');
      if (!ancestor || (ancestor.getAttribute('data-id') !== ancestorId && ancestor.id !== ancestorId)) return false;
    }
    return true;
  });
  if (candidates.length !== 1) {
    return {ok: false, reason: 'stale_reference', candidates: candidates.length};
  }
  arr[index] = candidates[0];
  return {ok: true, rebound: true, candidates: 1, tag: String(candidates[0].tagName).toLowerCase()};
}
