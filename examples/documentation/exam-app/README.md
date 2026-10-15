# exam-app

Answer a fixed two-question exam. The server grades and stores each unique attempt.

Prerequisites: [Axiom CLI 0.149.0](https://docs.axiomcore.dev/getting-started/installation/),
macOS arm64, a browser, and two terminals. Install the compatible web host:

```bash
axiom ui host install --target web --version 0.6.12 --non-interactive
```

From this directory, start the backend:

```bash
unset DATABASE_URL
axiom check backend/main.acore
axiom run backend/main.acore --port 8082
```

Keep the printed development key local. In another terminal in this directory:

```bash
axiom run frontend/main.acore --target web --port 5174
```

Open http://127.0.0.1:5174, paste the key, and select **Connect**. Submit a new
record, reload and reconnect, then restart the backend to confirm persistence.
Submitting the same attempt ID twice returns 409.

The manifests bind the frontend to the local backend contract. Keep them with
the source. If you edit an imported backend file, stop the processes, run
`axiom build backend/main.acore`, then restart both. Change the database project
version before applying a schema change.

This is a local teaching app with one development identity and tenant. It has
no production login or deployment configuration. The exam is untimed, has two fixed questions, and has no instructor roles or publishing workflow.

Stop both processes with Ctrl-C. Data remains in this directory's local state.
Remove the disposable directory only when its SQLite data is no longer needed.
Read the [complete guide](https://docs.axiomcore.dev/examples/exam-app/run/) for source details,
error cases, and support boundaries.
