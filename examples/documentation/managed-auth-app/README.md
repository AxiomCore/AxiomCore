# managed-auth-app

Sign in with Axiom managed authentication from an Acore web app, then call a
guarded backend that reads the verified user with `Principal.*`.

What it shows:

- `use auth Auth from "axiom"` and the built-in `axiom.auth/v1` operations:
  password, email code, Google and the hosted sign-in page.
- `sensitive state` for the password and the code. The compiler refuses to
  render, store, emit or log them (AUI460 to AUI466).
- `session_actions(sign_in: [byPassword, byCode])` with
  `session sign_in(<name>)` selecting one sign-in.
- A guarded backend endpoint that receives the access token
  (`auth(required: true, methods: [AuthMethodDef(type: "bearer", ...)])`) and
  returns `Principal.subject` and `Principal.sid`.

Setup:

1. Enable authentication for the project's `development` environment in the
   dashboard. Add `http://127.0.0.1:5173/auth/callback` to the redirect URLs
   and `http://127.0.0.1:5173` to the allowed origins.
2. Write the issuer into the frontend manifest:
   `axiom auth config pull --environment development --manifest frontend/AxiomDeps.toml --write`.
3. Check the manifest against the environment:
   `axiom auth check --manifest frontend/AxiomDeps.toml --backend backend/application.acore`
   (it also reports guarded endpoints without an `auth(...)` clause).
4. Build the backend (`axiom build backend/application.acore`) and run it with
   an `axiomAuth` deployment for the same environment (the server deployment
   scaffold writes one from `--project-id` and `--environment-id`).
5. Run the frontend: `axiom run frontend/main.acore --target web --port 5173`.

The refresh token is kept in memory, so a reload signs the user out. Set
`storage = "session"` in `[auth]` to keep it for the tab. Managed auth runs on
the web host only.
