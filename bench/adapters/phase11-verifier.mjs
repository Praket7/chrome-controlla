#!/usr/bin/env node
import { createHash } from 'node:crypto';
const chunks=[]; for await (const chunk of process.stdin) chunks.push(chunk); const request=JSON.parse(Buffer.concat(chunks).toString());
const input=request.input ?? request; const base=process.env.PHASE11_FIXTURE_BASE_URL; if(!base) throw new Error('PHASE11_FIXTURE_BASE_URL is required');
const response=await fetch(`${base}/__state?task_id=${encodeURIComponent(input.task_id)}`,{signal:AbortSignal.timeout(input.deadline_ms??60000)});
const state=await response.json(); const passed=state.task_id===input.task_id && state.completed===true;
const evidence={task_id:state.task_id,completed:state.completed,event_count:state.events?.length??0,last_kind:state.events?.at(-1)?.kind??null};
console.log(JSON.stringify({passed,evidence_hash:createHash('sha256').update(JSON.stringify(evidence)).digest('hex'),evidence}));
