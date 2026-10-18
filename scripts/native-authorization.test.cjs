const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('extensions/chrome-controlla/background.js','utf8');
const match = source.match(/function nativeLeaseAuthorizes\([\s\S]*?(?=\nfunction |\nasync function )/);
assert.ok(match, 'native authorization must check live lease registration');
function fixture() {
  const lease={ id:'a', generation:2, tabs:new Set([1]), unrestricted:false };
  const context={nativeGeneration:3,nativeLeases:new Map([['a',lease]]),nativeAttachedTabs:new Set([1])};
  vm.runInNewContext(match[0]+'\nglobalThis.authorize=nativeLeaseAuthorizes',context);
  return {context,lease};
}
test('released or replaced leases cannot authorize captured commands',()=>{
  const {context:c,lease}=fixture();assert.equal(c.authorize('a',lease,1,3),true);
  c.nativeLeases.delete('a');assert.equal(c.authorize('a',lease,1,3),false);
  c.nativeLeases.set('a',{...lease});assert.equal(c.authorize('a',lease,1,3),false);
});
test('unknown explicit lease cannot borrow another attached tab',()=>{
  const {context:c}=fixture();assert.equal(c.authorize('missing',undefined,1,3),false);
});
test('generation and target identities must still match',()=>{
  const {context:c,lease}=fixture();assert.equal(c.authorize('a',lease,1,2),false);
  assert.equal(c.authorize('a',lease,2,3),false);
});
