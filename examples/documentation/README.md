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

Verified on macOS arm64 with the published CLI 0.149.0. Acore UI examples need
signed web host 0.6.12. This does not establish support for other distributions.
The existing-stack samples pin their published client generator and runtime.

From `docs`, run `pnpm examples:package` to refresh deterministic source archives
and `pnpm examples:check` to reject stale or unsafe archives.
