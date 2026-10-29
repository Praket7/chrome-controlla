function(index) {
  // A wrapper control (`div[role=group]`, an editor shell, a clickable canvas
  // host) is not typeable itself. Resolve it to the one editable descendant so
  // the text route can proceed against the real target instead of refusing
  // with `not_editable`.
  //
  // The snapshot remote object is either the retained `nodes` array or the
  // element itself, depending on the dispatch site. Both shapes are handled:
  // when `nodes` exists the binding is rewritten in place, and in every case
  // the resolved element is returned so the caller can retarget the remote
  // object it passes to the editor guard.
  const arr = this.nodes || null;
  const e = arr ? arr[index] : this;
  if (!e || e.nodeType !== 1) return null;
  const editable = n => {
    if (!n || n.nodeType !== 1) return false;
    const tag = String(n.tagName).toUpperCase();
    if (n.matches(':disabled,[aria-disabled="true"],[aria-readonly="true"],[readonly]')) return false;
    if (tag === 'TEXTAREA') return true;
    if (tag === 'INPUT') {
      return ['text', 'search', 'email', 'url', 'tel', ''].includes((n.getAttribute('type') || '').toLowerCase());
    }
    // A role alone does not make a node typeable: a `role=textbox` shell
    // wrapping a real editor is exactly what this rung resolves.
    return n.isContentEditable === true
      || (n.getAttribute('role') === 'textbox' && String(n.getAttribute('contenteditable') || '').toLowerCase() === 'true');
  };
  let target = editable(e) ? e : null;
  if (!target) {
    const found = [];
    let visited = 0;
    const visit = (root, depth) => {
      if (depth > 6 || found.length > 4 || visited > 2000) return;
      let children = [];
      try { children = Array.from(root.querySelectorAll(':scope > *')); } catch (_) { children = []; }
      for (let i = 0; i < children.length; i++) {
        const child = children[i];
        visited += 1;
        if (visited > 2000) return;
        if (editable(child)) found.push(child);
        if (child.shadowRoot) visit(child.shadowRoot, depth + 1);
        visit(child, depth + 1);
      }
    };
    visit(e, 0);
    if (found.length !== 1) return null;
    target = found[0];
  }
  if (arr) arr[index] = target;
  return target;
}
