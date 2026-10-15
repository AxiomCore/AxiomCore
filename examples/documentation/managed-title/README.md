# managed-title

A small Acore backend returning trimmed title or typed 422 failure. Requires published CLI 0.149.0 on
macOS arm64. Run from this directory:

```bash
axiom check main.acore
axiom build main.acore --out backend.axiom
axiom server deployment backend.axiom backend.server.json --output deployment.json
axiom run main.acore --deployment deployment.json --port 8085
```

In another terminal:

```bash
curl -fsS http://127.0.0.1:8085/tasks -H 'Content-Type: application/json' -d '"  Read the guide  "'
```

The request body is a JSON string. Whitespace-only input returns the declared 422 failure; no record is persisted.
The endpoint is public and has no external effects or provider grants. Review
identity and grants before adapting it to protected data or effects. Stop the
server with Ctrl-C. Remove the disposable directory to remove generated files.
