#!/usr/bin/env node
import { createHash } from 'node:crypto';
const chunks=[]; for await (const chunk of process.stdin) chunks.push(chunk); const request=JSON.parse(Buffer.concat(chunks).toString());
const input=request.input ?? request; const base=process.env.PHASE11_FIXTURE_BASE_URL; if(!base) throw new Error('PHASE11_FIXTURE_BASE_URL is required');
const response=await fetch(`${base}/__state?task_id=${encodeURIComponent(input.task_id)}`,{signal:AbortSignal.timeout(input.deadline_ms??60000)});
const state=await response.json();
const eventKinds=Array.isArray(state.events)?state.events.map(event=>event.kind):[];
const missingReason='fixture event log is writable by the acting browser and is not an independent task acceptance oracle';
const evidence={task_id:state.task_id,task_class:input.task_class,oracle_status:'missing',missing_reason:missingReason,event_count:eventKinds.length,last_kind:eventKinds.at(-1)??null};
console.log(JSON.stringify({passed:false,missing_reason:missingReason,evidence_hash:createHash('sha256').update(JSON.stringify(evidence)).digest('hex'),evidence}));
