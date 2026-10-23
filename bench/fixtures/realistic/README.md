# Fixture acceptance contract

The fixture service accepts actual form values at `POST /accept/form?task=…&run=…`
and raw upload bytes at `POST /accept/upload?task=…&run=…&filename=…`. Upload
size and SHA-256 are calculated by the server. The page uses these endpoints;
`/__event` is diagnostic only and never changes accepted state.

Set a random `PHASE11_ORACLE_TOKEN` in the fixture server, reset command and
verifier command environments. `POST /__reset` and `GET /__state` require
`Authorization: Bearer …`. Reset requires bounded `task_id` and `run_id`; the
runner creates a new run UUID per attempt. Readback and submission must match
that run. No token is rendered into HTML. The runner removes the oracle token
and state-file path from the acting adapter's environment.

Only the basic `form-guarded-fill` and `guarded-upload` tasks receive runner-owned
acceptance predicates. Form values must exactly match the predicate; uploads
must match filename, size and digest. Unsupported classes and specialized
scenarios stay unqualified. The convenience command adapter remains unqualified;
this service does not supply real client/browser execution or timing hooks.

Writes are serialized and atomically renamed before acceptance is acknowledged.
A persistence failure marks the state unaccepted. Atomic replacement is not a
power-loss durability guarantee. Inputs are bounded (4,096 characters per form
field, 12 MiB uploads, 256 retained tasks). This disposable loopback fixture
trusts its host and privileged harness: it is independent of page-emitted events,
not an isolation boundary against an arbitrary same-user process with filesystem
or process inspection rights. Use a separate service identity if that adversary
is in scope. Service state verifies application acceptance, not which browser
interaction or API was used to submit it.
