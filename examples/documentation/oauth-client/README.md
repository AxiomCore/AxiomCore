# oauth-client (E11)

An OAuth application signing in through the Axiom OIDC issuer: authorization code with PKCE, ID token verified, consent and logout.

Status: lead row. Project `axq-w8x-auth`, wave 1 (examples matrix 3.1).
Depends on: W8.1 (A1g, A1h merged); `axiom auth oauth-apps` from lane D2a.

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh oauth-client            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh oauth-client
```

Sign in with the relying party (Python 3.9 or later, standard library; EdDSA ID tokens also need
the `cryptography` package), after registering a public client with redirect URI
`http://127.0.0.1:5178/callback`:

```sh
python3 -B client/rp.py --issuer https://auth.axiomcore.dev/<project-id>/<environment-id> --client-id axcl_...
```

It opens the browser, waits for the callback on loopback, and prints `PASS`/`FAIL` per check and
the ID token's claim names; tokens are written only to `--out` (0600 files), never printed.

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
