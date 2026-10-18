const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('extensions/chrome-controlla/background.js', 'utf8');
const match = source.match(/async function manageTabTopology\([\s\S]*?(?=\nasync function dispatchCommand\()/);
assert.ok(match, 'production tab management implementation must exist');
function fixture() {
  const tabs = new Map([[1, { id:1, windowId:10, index:0, groupId:-1, active:false, pinned:false }],
    [2, { id:2, windowId:10, index:1, groupId:-1, active:false, pinned:false }]]);
  const groups = new Map(); const calls = [];
  const documents = new Map([[1, 'doc1'], [2, 'doc2']]);
  const chrome = {
    tabs: {
      async get(id) { if (!tabs.has(id)) throw Error('missing'); return { ...tabs.get(id) }; },
      async query({ groupId } = {}) { return [...tabs.values()].filter(t=>groupId === undefined || t.groupId===groupId); },
      async group({ tabIds, groupId }) { calls.push('group'); const id=groupId??20; groups.set(id, groups.get(id)||{id,windowId:10}); for(const tid of tabIds)tabs.get(tid).groupId=id; return id; },
      async ungroup(ids) { calls.push('ungroup'); for(const id of ids)tabs.get(id).groupId=-1; },
      async move(ids, options) { calls.push('move'); ids.forEach((id,i)=>Object.assign(tabs.get(id),{windowId:options.windowId??tabs.get(id).windowId,index:options.index===-1?i:options.index+i})); },
      async remove(ids) { calls.push('close'); for(const id of ids)tabs.delete(id); },
    },
    windows: { async get(id) { return {id, focused:false, incognito:false, type:'normal'}; } },
    tabGroups: {
      async get(id) { if(!groups.has(id))throw Error('missing group'); return {...groups.get(id)}; },
      async update(id, data) { calls.push('update');Object.assign(groups.get(id),data);return {...groups.get(id)}; },
    },
  };
  const context = { chrome, Set, Map, Number, Object, Array, String, Error, JSON,
    nativePageIdentity: async id=>({document_id:documents.get(id)}),
    pairedTabsInTransit:new Set(), bumpPairedTabTopology(){}, bindPairedTabTopology(){},
  };
  vm.runInNewContext(match[0]+'\nglobalThis.manage=manageTabTopology;', context);
  return { tabs,groups,calls,documents,chrome, run: params=>context.manage(params,()=>true) };
}
const targets=[{tab_id:'1',document_id:'doc1'},{tab_id:'2',document_id:'doc2'}];
test('group and style exact document-bound tabs, then ungroup', async()=>{
  const f=fixture();const result=await f.run({kind:'group',targets,title:'Research',color:'blue',collapsed:true,background:true});
  assert.equal(result.status,'verified');assert.equal(result.group_id,20);
  assert.equal(f.groups.get(20).title,'Research');
  assert.equal((await f.run({kind:'ungroup',targets,background:true})).status,'verified');
  assert.equal(f.tabs.get(1).groupId,-1);
});
test('move preserves input order and verifies destination window',async()=>{
  const f=fixture();const r=await f.run({kind:'move',targets,index:3,window_id:11,background:true});
  assert.equal(r.status,'verified');assert.equal(f.tabs.get(1).index,3);assert.equal(f.tabs.get(2).index,4);
});
test('close confirms removal of exact selected tabs',async()=>{
  const f=fixture();assert.equal((await f.run({kind:'close',targets,background:false})).status,'verified');assert.equal(f.tabs.size,0);
});
test('stale document refuses before changing topology',async()=>{
  const f=fixture();f.documents.set(2,'replacement');const r=await f.run({kind:'close',targets,background:false});
  assert.equal(r.status,'not_dispatched');assert.deepEqual(f.calls,[]);
});
test('background move refuses active tabs',async()=>{
  const f=fixture();f.tabs.get(1).active=true;const r=await f.run({kind:'move',targets,index:0,background:true});
  assert.equal(r.status,'not_dispatched');assert.deepEqual(f.calls,[]);
});
test('unknown or duplicate target refuses before dispatch',async()=>{
  for(const selected of [[targets[0],targets[0]],[{tab_id:'1e0',document_id:'doc1'}]]){
    const f=fixture();assert.equal((await f.run({kind:'close',targets:selected,background:false})).status,'not_dispatched');assert.deepEqual(f.calls,[]);
  }
});
test('partial group mutation never reports not_dispatched',async()=>{
  const f=fixture();f.chrome.tabGroups.update=async()=>{throw Error('lost reply')};
  const r=await f.run({kind:'group',targets,title:'Research',background:false});
  assert.equal(r.status,'unknown');assert.equal(r.dispatch_may_have_occurred,true);assert.deepEqual(f.calls,['group']);
});
test('failed removal readback is not proof of closure',async()=>{
  const f=fixture();f.chrome.tabs.remove=async()=>{};assert.equal((await f.run({kind:'close',targets,background:false})).status,'unknown');
});

test('closure cannot be verified from a failing Chrome inventory',async()=>{
  const f=fixture();f.chrome.tabs.query=async()=>{throw Error('browser disconnected')};
  assert.equal((await f.run({kind:'close',targets,background:false})).status,'unknown');
});
