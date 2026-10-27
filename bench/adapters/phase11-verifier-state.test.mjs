import { test } from 'node:test';
import assert from 'node:assert/strict';
import { evaluateAcceptance } from './phase11-verifier-state.mjs';
const input = { task_id:'form-one', run_id:'run-one', task_class:'forms', acceptance:{kind:'form',name:'Alice',notes:'Saved'} };
const state = { task_id:input.task_id,run_id:input.run_id,persisted:true,form:{name:'Alice',notes:'Saved'} };
test('extraction requires the oracle nonce from the exact reset run',()=>{
 const request={task_id:'structured-extraction',run_id:'run-one',task_class:'extraction',acceptance:{kind:'extraction',row:'record-42',status:'older'}};
 const nonce='a'.repeat(32), state={task_id:request.task_id,run_id:request.run_id,extraction_nonce:nonce};
 assert.equal(evaluateAcceptance(request,state,{extraction_value:`record-42|older|${nonce}`}).passed,true);
 for(const [s,result] of [[state,{}],[state,{extraction_value:'record-42|older'}],[{...state,run_id:'other'},{extraction_value:`record-42|older|${nonce}`}],[{...state,extraction_nonce:'b'.repeat(32)},{extraction_value:`record-42|older|${nonce}`}]]) assert.equal(evaluateAcceptance(request,s,result).passed,false);
});
test('server persisted form qualifies exact bound run only',()=>{
 assert.equal(evaluateAcceptance(input,state).passed,true);
 for(const bad of [{...state,persisted:false},{...state,run_id:'old'},{...state,form:null},{...state,form:{name:'Other',notes:'Saved'}},{...state,events:[{kind:'form_saved',completed:true}],form:null}]) assert.equal(evaluateAcceptance(input,bad).passed,false);
});
test('upload acceptance requires exact server byte digest',()=>{
 const upload={kind:'upload',filename:'a.txt',size:3,sha256:'a'.repeat(64)};
 const request={...input,task_class:'upload',acceptance:upload};
 assert.equal(evaluateAcceptance(request,{...state,upload}).passed,true);
 assert.equal(evaluateAcceptance(request,{...state,upload:{...upload,sha256:'b'.repeat(64)}}).passed,false);
});
test('unsupported and unbounded predicates remain unqualified',()=>{
 assert.equal(evaluateAcceptance({...input,task_class:'extraction'},state).evidence.oracle_status,'missing');
 assert.equal(evaluateAcceptance({...input,scenario:'cross-frame-rich-editor'},state).passed,false);
 assert.equal(evaluateAcceptance({...input,acceptance:{...input.acceptance,extra:'untrusted'}},state).passed,false);
 assert.equal(evaluateAcceptance({...input,acceptance:{...input.acceptance,name:'a'.repeat(4097)}},state).passed,false);
});
