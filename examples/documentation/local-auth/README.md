# local-auth (E12)

Develop sign-in locally with no Cloud account, no network and no hosted database, then promote
the auth settings to a Cloud environment.

What it shows:

- `axiom auth emulate`: a loopback auth server for this directory. Issuer
  `http://127.0.0.1:<port>/local/<project>`; storage is a disposable SQLite file under
  `.axiom/auth-local/` with the same `axiom-auth` stream the hosted environments use;
  signing keys are generated on every start.
- A mail viewer at `/local/mail` (JSON at `/local/mail.json`): sign-up and sign-in codes are
  captured there, never sent.
- A mock OIDC issuer at `/local/oidc` that signs synthetic identities only
  (`--identity jo@example.com`). Every provider in the local configuration (here `google`)
  signs in through it.
- A guarded backend run by the local runtime, which verifies the emulator's tokens with a
  `jwt` EdDSA provider (`axiom auth emulate deployment`). Cloud runtimes and issuers refuse
  them: the issuer is local.
- `axiom auth config export --from local`: public settings and secret references only.
  Loopback callbacks and origins, the mock provider, users, sessions, keys and mail never
  leave the directory. `diff` and `push --from local` promote the same configuration.

Run locally:

Tokens and the synthetic password go to a private directory outside the project (`$work`),
never into the source tree:

```sh
work=$(mktemp -d)                                  # mode 700; delete it when done
axiom auth emulate --identity jo@example.com      # keep it running; prints issuer, mail, oidc
python3 -B flow.py password .axiom/auth-local/state.json "$work"   # sign-up, codes, refresh
python3 -B flow.py oidc .axiom/auth-local/state.json "$work"       # mock OIDC sign-in
cd backend && axiom build application.acore && cd ..
axiom server deployment backend/.axiom/contracts/backend.axiom backend/.axiom/contracts/backend.server.json -o "$work/deployment.json" --static-key
axiom auth emulate deployment "$work/deployment.json"
AXIOM_AUTH_LOCAL_PUBLIC_KEY="$(cat .axiom/auth-local/public.pem)" \
  axiom server run backend/.axiom/contracts/backend.axiom backend/.axiom/contracts/backend.server.json --deployment "$work/deployment.json" --listen 127.0.0.1:8080
axiom auth config export --from local --output "$work/config.json"
axiom auth emulate reset                           # after stopping the emulator
rm -rf "$work"
```

`auth/config.local.json` holds the local settings (`axiom-auth-config/v1`, secrets as
`{"ref": "<SECRET_NAME>"}` only). Without it the emulator uses defaults for
`http://127.0.0.1:5173`.

Promote (lead, production): `axiom auth config diff --from local --to env:development` names
the environment, each item's security impact and the runtime target; `push` refuses until every
widening and credential item is approved in `auth/config.approvals.json`.

`check.sh` runs all local steps (`PASS`, `FAIL` and `LEAD` lines) and, with
`AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug>`, the promotion steps:

```sh
bash ../../../tools/w8D2/run-row.sh local-auth
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh local-auth
```

Not emulated: magic links, password reset, invitations, MFA, passkeys, hosted pages and
OAuth applications (they answer `AXAUTH_NOT_FOUND`). Use a preview branch with captured mail
for those.
