# Documentation examples

Runnable source for the [AxiomCore documentation](https://docs.axiomcore.dev/examples/).
The examples use synthetic local data. Each directory has its own setup and cleanup steps.

| Example | Stack | Guide |
| --- | --- | --- |
| `task-app` | Acore UI, Acore API, SQLite | [First app](https://docs.axiomcore.dev/getting-started/quickstart/) |
| `exam-app` | Acore UI, server-side scoring, SQLite | [Exam tutorial](https://docs.axiomcore.dev/examples/exam-app/) |
| `hello-api` | Pure Acore endpoint | [Backend quickstart](https://docs.axiomcore.dev/getting-started/quickstart-backend/) |
| `managed-title` | Acore normalization and typed failure | [Managed starter](https://docs.axiomcore.dev/backend/managed-starter/) |
| `fastapi-web` | Python/FastAPI and ATMX web | [FastAPI integration](https://docs.axiomcore.dev/guides/fastapi-to-web/) |
| `go-web` | Go/Chi and ATMX web | [Go integration](https://docs.axiomcore.dev/guides/go-to-web/) |

W8.7 qualification examples (examples matrix rows E1 to E16). Each has a `check.sh`
that runs its offline steps from the installed CLI and stops before production steps
unless `AXIOM_W8_APPLY=yes`; `tools/w8D2/run-row.sh` runs a row from its packaged archive
in a clean prefix and HOME with canary scans (counts and file names only).

| Example | Rows | Locally verified |
| --- | --- | --- |
| `managed-auth-app` | E1, E2 | backend build |
| `hosted-persistence` | E3, E4 | schema check, backend build |
| `rbac-tenants` | E5 | backend build (policy check needs the W8 CLI) |
| `external-issuer` | E6 | backend build |
| `byo-turso` | E7 | schema check |
| `upgrade-retains-libsql` | E8 | schema check |
| `convert-to-postgres` | E9 | both schema checks |
| `passkeys-mfa` | E10 | needs the W8 CLI (assurance guards) |
| `oauth-client` | E11 | relying-party unit tests |
| `local-auth` | E12 | complete locally: emulator, mail viewer, mock OIDC, a local runtime verifying its tokens, export (promotion is the lead's) |
| `fastapi-auth` | E13 | verifier and API tests, web build from the tarball |
| `linux-backend` | E14 | build, axiomAuth scaffold holds no credential, native validate |
| `sandbox-refusal` | E15 | complete (Workers refuses axiomAuth and Principal.*) |
| `preview-branch` | E16 | schema check |

Python examples need Python 3.10 or later; web clients need Node 24 or later.

Verified on macOS arm64 with the published CLI 0.149.0. Acore UI examples need
signed web host 0.6.12. This does not establish support for other distributions.
The existing-stack samples pin their published client generator and runtime.

From `docs`, run `pnpm examples:package` to refresh deterministic source archives
and `pnpm examples:check` to reject stale or unsafe archives.
