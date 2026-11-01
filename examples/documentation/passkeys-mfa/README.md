# passkeys-mfa (E10)

TOTP and recovery codes, passkeys through Chrome's virtual authenticator, reauthentication, an assurance-guarded endpoint.

Status: lead row. Project `axq-w8x-auth`, wave 1 (examples matrix 3.1).
Depends on: W8.1; G1 (UI host on http://localhost:<port>, RP ID localhost); auth-web 0.2.0.

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh passkeys-mfa            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh passkeys-mfa
```

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
