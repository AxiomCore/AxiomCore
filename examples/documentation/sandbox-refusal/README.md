# sandbox-refusal (E15)

A sandboxed target (Cloudflare Workers v1) is refused authentication capability. Nothing
is allocated and nothing is contacted: `check.sh` builds two small backends, scaffolds an
`axiomAuth` deployment with synthetic IDs and validates it for `cloudflare-workers-local`.

Assertions:

1. `backend-status/`: a guarded action that reads no principal. Validation fails with
   `AXSERVER_TARGET` at `deployment.authentication`, requirement `HS256 JWT`
   (`axiomAuth` needs a native or container target). The same contract validates for
   `native-server`.
2. `backend/`: a managed action reading `Principal.*` is refused on Workers on its own
   (`Principal.* is unavailable on Workers`).

Not yet covered here (lead rows): a database binding on Workers (path
`resources.<name>.provider`, requirement `httpClient`) and a sandbox extension binding
outside the private plan's extensions.

Run: `bash check.sh` with the installed CLI on `PATH`.
