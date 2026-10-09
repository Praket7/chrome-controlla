function(q, index) {
  if (q.kind === 'navigation') return false;
  const es = q.selector ? [...document.querySelectorAll(q.selector)] : [this?.nodes?.[index]].filter(Boolean);
  if (es.length !== 1) return false;
  const e = es[0], s = getComputedStyle(e), b = e.getBoundingClientRect();
  if (!e.isConnected || e.querySelector('[data-masked],[data-requires-trusted],input[type=password],input[type=file],input[type=hidden]') || e.closest('[data-masked],[data-requires-trusted],[hidden],[inert]') || (e instanceof HTMLInputElement && ['password','hidden','file'].includes(e.type))) return false;
  const visible = b.width>0 && b.height>0 && s.visibility==='visible' && s.display!=='none';
  if (q.kind === 'visible') return visible;
  if (q.kind === 'text') return visible && (e.innerText || '').replace(/\s+/g,' ').trim() === q.text.replace(/\s+/g,' ').trim();
  if (q.kind === 'expanded') return visible && e.getAttribute('aria-expanded') === 'true';
  if (q.kind === 'selected') return visible && (e.getAttribute('aria-selected') === 'true' || e.getAttribute('aria-checked') === 'true');
  return false;
}
