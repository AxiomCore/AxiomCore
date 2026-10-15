# task-app

Create, update, and delete tasks backed by real SQLite storage.

Prerequisites: [Axiom CLI 0.149.0](https://docs.axiomcore.dev/getting-started/installation/),
macOS arm64, a browser, and two terminals. Install the compatible web host:

```bash
axiom ui host install --target web --version 0.6.12 --non-interactive
```

From this directory, start the backend:

```bash
unset DATABASE_URL
axiom check backend/sqlite.acore
axiom run backend/sqlite.acore --port 8080
```

Keep the printed development key local. In another terminal in this directory:

```bash
axiom run frontend/main.acore --target web --port 5173
```

Open http://127.0.0.1:5173, paste the key, and select **Connect**. Submit a new
record, reload and reconnect, then restart the backend to confirm persistence.
Submitting the same title twice returns 409.

The manifests bind the frontend to the local backend contract. Keep them with
the source. If you edit an imported backend file, stop the processes, run
`axiom build backend/sqlite.acore`, then restart both. Change the database project
version before applying a schema change.

This is a local teaching app with one development identity and tenant. It has
no production login or deployment configuration. List responses are bounded to 100 tasks; this is not a multi-user task service.

Stop both processes with Ctrl-C. Data remains in this directory's local state.
Remove the disposable directory only when its SQLite data is no longer needed.
Read the [complete guide](https://docs.axiomcore.dev/getting-started/quickstart/) for source details,
error cases, and support boundaries.
