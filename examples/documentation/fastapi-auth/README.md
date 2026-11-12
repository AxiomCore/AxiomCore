# fastapi-auth (E13)

An existing FastAPI backend that verifies Axiom access tokens, and a plain TypeScript web
client that signs in with `@axiomcore/auth-web`.

What it shows:

- `backend/axiom_verify.py`: EdDSA (Ed25519) verification against the issuer's
  `/.well-known/jwks.json` with the algorithm fixed by the server (never `none`, never
  HS256), exact `iss` and `aud`, lifetime, `token_use`, and refusal of OAuth application
  tokens (`client_id`, `scope`). Refusals carry a fixed reason code, never token content.
- `backend/main.py`: a `/me` endpoint behind that check.
- `frontend/`: hosted sign-in, `authorizedFetch` (one refresh and one retry on
  `401 authorization_changed`), session storage per tab.

Local check (no account needed): `AXIOM_AUTH_WEB_TGZ=<packed tarball> bash check.sh`.

With an environment: set `AXIOM_ISSUER` and `AXIOM_AUDIENCE` from
`axiom auth config pull --environment development`, add `http://127.0.0.1:5176` to the
allowed origins and `http://127.0.0.1:5176/auth/callback` to the redirect URLs, then
`uvicorn main:app --port 8000` in `backend/` and serve `frontend/dist` on port 5176.

Status: local part verified; the real sign-in is a lead row (project `axq-w8x-issuer`, wave 1).
