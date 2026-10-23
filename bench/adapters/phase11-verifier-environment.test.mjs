import {test} from 'node:test';
import assert from 'node:assert/strict';
import {actingAdapterEnvironment} from '../runners/phase11-runner.mjs';
test('acting adapter never inherits reset/readback token or fixture state path',()=>{
 const privileged={PATH:'/bin',PHASE11_ORACLE_TOKEN:'secret',PHASE11_STATE_FILE:'/private/state',PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1'};
 const acting=actingAdapterEnvironment(privileged);
 assert.equal(acting.PHASE11_ORACLE_TOKEN,undefined);assert.equal(acting.PHASE11_STATE_FILE,undefined);
 assert.equal(acting.PATH,'/bin');assert.equal(acting.PHASE11_FIXTURE_BASE_URL,privileged.PHASE11_FIXTURE_BASE_URL);
 assert.equal(privileged.PHASE11_ORACLE_TOKEN,'secret');
});
