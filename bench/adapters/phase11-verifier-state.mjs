import { createHash } from 'node:crypto';
export function validAcceptance(value) {
 if (!value || typeof value !== 'object') return false;
 if (value.kind === 'extraction') return Object.keys(value).sort().join(',') === 'kind,row,status' && value.row === 'record-42' && value.status === 'older';
 if (Object.keys(value).some(key=>!(value.kind==='form'?['kind','name','notes']:['kind','filename','size','sha256']).includes(key))) return false;
 if (value.kind === 'form') return ['name','notes'].every(key=>typeof value[key]==='string' && value[key].length<=4096) && value.name.length>0;
 return value.kind==='upload' && typeof value.filename==='string' && value.filename.length>0 && value.filename.length<=255 && Number.isSafeInteger(value.size) && value.size>0 && value.size<=12*1024*1024 && /^[a-f0-9]{64}$/.test(value.sha256);
}
export function evaluateAcceptance(input,state,actingResult) {
 const supported=!input.scenario && ((input.task_class==='forms' && input.acceptance?.kind==='form') || (input.task_class==='upload' && input.acceptance?.kind==='upload') || (input.task_class==='extraction' && input.acceptance?.kind==='extraction'));
 const evidence={task_id:input.task_id,run_id:input.run_id??null,task_class:input.task_class,oracle_status:supported?(input.acceptance?.kind==='extraction'?'run_bound_extraction_value':'server_state'):'missing'};
 const missing_reason=supported?undefined:'fixture event log is writable by the acting browser and is not an independent task acceptance oracle; scenario has no supported state predicate';
 const bound=typeof input.run_id==='string' && input.run_id.length>0 && state?.run_id===input.run_id && state?.task_id===input.task_id && (input.acceptance?.kind==='extraction' || state?.persisted===true);
 const expected=input.acceptance; const actual=state?.[expected?.kind];
 const passed=Boolean(supported && bound && validAcceptance(expected) && (expected.kind==='extraction' ? /^[a-f0-9]{32}$/.test(state?.extraction_nonce ?? '') && actingResult?.extraction_value === `${expected.row}|${expected.status}|${state.extraction_nonce}` : actual && Object.keys(expected).every(key=>key==='kind'||actual[key]===expected[key])));
 if (expected?.kind==='extraction') evidence.browser_use_proof='trusted_adapter_trace_only';
 evidence.accepted=passed;
 evidence.persisted=state?.persisted===true;
 evidence.state_hash=createHash('sha256').update(JSON.stringify({form:state?.form??null,upload:state?.upload??null,extraction_nonce:state?.extraction_nonce??null})).digest('hex');
 return {passed,...(missing_reason?{missing_reason}:{}),evidence_hash:createHash('sha256').update(JSON.stringify(evidence)).digest('hex'),evidence};
}
