# external-issuer (E6)

Trusted external token exchange and the Supabase, Auth0, Clerk presets; Microsoft real, Apple mock.

Status: lead row. Project `axq-w8x-issuer`, wave 1 (examples matrix 3.1).
Depends on: W8.2; O1 test apps (G2); `axiom auth trusted-issuers` from lane D2a.

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh external-issuer            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh external-issuer
```

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
