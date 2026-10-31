// CDP function bodies for text transactions. Kept apart from the transaction
// orchestration so the dispatch logic has one readable home and the injected
// source stays reviewable on its own.

export const CONNECTED_PROBE = `function(index){const e=this.nodes?this.nodes[index]:this;return {ok:!!e&&e.isConnected===true}}`;

export const DOM_INSERT = `function(chunk){
  const e=this;if(!e||!e.isConnected)return {ok:false,reason:'target_stale'};
  const tag=String(e.tagName||'').toUpperCase();
  const rich=e.nodeType===1&&tag!=='INPUT'&&tag!=='TEXTAREA'&&(e.isContentEditable===true||(e.getAttribute&&e.getAttribute('role')==='textbox'));
  // Focus is per-document, so it can be restored while the tab itself sits in
  // the background. preventScroll keeps a background insert from moving the
  // page the user is looking at in another tab.
  try{e.focus({preventScroll:true})}catch(_){try{e.focus()}catch(_){}}
  if(tag==='TEXTAREA'||tag==='INPUT'){
    if(typeof e.setRangeText==='function'){try{e.setRangeText(chunk,e.value.length,e.value.length,'end')}catch(_){e.value=(e.value||'')+chunk}}
    else e.value=(e.value||'')+chunk;
    e.dispatchEvent(new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType:'insertText',data:chunk}));
    e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:chunk}));
    return {ok:String(e.value||'').endsWith(chunk),value:e.value,length:String(e.value||'').length};
  }
  if(!rich)return {ok:false,reason:'not_editable'};
  // Append semantics: an editor that lost focus may have left its caret in a
  // paragraph the user clicked, and execCommand would insert there. Collapse
  // the caret to the end of the editing surface first.
  const doc=e.ownerDocument;
  const caretToEnd=()=>{try{const sel=doc.getSelection(),range=doc.createRange();range.selectNodeContents(e);range.collapse(false);sel.removeAllRanges();sel.addRange(range)}catch(_){}};
  caretToEnd();
  let inserted=false;
  try{inserted=doc.execCommand('insertText',false,chunk)}catch(_){inserted=false}
  if(!inserted){
    try{
      const sel=doc.getSelection(),range=doc.createRange();
      range.selectNodeContents(e);range.collapse(false);
      const text=doc.createTextNode(chunk);range.insertNode(text);
      range.setStartAfter(text);range.collapse(true);
      sel.removeAllRanges();sel.addRange(range);
      e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:chunk}));
      inserted=true;
    }catch(_){inserted=false}
  }
  const value=e.innerText??e.textContent??'';
  return {ok:inserted,value,length:value.length};
}`;

export const REBIND_EDITOR = identity => `(()=>{
  const tag=${JSON.stringify(identity?.tag ?? "")},role=${JSON.stringify(identity?.role ?? "")},label=${JSON.stringify(identity?.label ?? "")},placeholder=${JSON.stringify(identity?.placeholder ?? "")},id=${JSON.stringify(identity?.id ?? "")};
  if(!tag)return null;
  const matches=n=>{
    if(!n||n.nodeType!==1)return false;
    if(String(n.tagName||'').toUpperCase()!==tag)return false;
    if(String(n.getAttribute('role')||'')!==role)return false;
    if(String(n.getAttribute('aria-label')||'').trim()!==label)return false;
    if(String(n.getAttribute('aria-placeholder')||n.getAttribute('data-placeholder')||'').trim()!==placeholder)return false;
    if(id&&n.id!==id)return false;
    return !n.matches(':disabled,[aria-disabled="true"],[aria-readonly="true"]');
  };
  const roots=[document],seen=new Set([document]);
  for(let depth=0,frontier=[document];depth<3&&frontier.length;depth+=1){
    const next=[];
    for(const doc of frontier){let frames=[];try{frames=Array.from(doc.querySelectorAll('iframe,frame'))}catch(_){frames=[]}
      for(const frame of frames){let child=null;try{child=frame.contentDocument}catch(_){child=null}
        if(child&&!seen.has(child)){seen.add(child);roots.push(child);next.push(child)}}}
    frontier=next;
  }
  const found=[];
  for(const doc of roots){try{for(const n of doc.querySelectorAll('textarea,input,[contenteditable="true"],[contenteditable=""],[role="textbox"]'))if(matches(n))found.push(n)}catch(_){}}
  return found.length===1?found[0]:null;
})()`;

export const NATIVE_SETTER = `function(expected,value){
  const e=this;
  if(!e||!e.isConnected)return {ok:false,reason:'target_stale'};
  if(e.value!==expected)return {ok:false,reason:'stale_value',value:e.value};
  const d=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value');
  d?.set?.call(e,value);
  e.dispatchEvent(new Event('input',{bubbles:true}));
  e.dispatchEvent(new Event('change',{bubbles:true}));
  return {ok:e.value===value,value:e.value,length:e.value.length};
}`;
