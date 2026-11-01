# local-auth (E12)

Local auth emulation (no network, no hosted database), mail viewer, mock OIDC, then config promotion to Cloud.

Status: lead row. Project `local, then axq-w8x-auth development`, wave 1 (examples matrix 3.1).
Depends on: lane D2a: `axiom auth emulate`, `axiom auth config export|diff|push` (section 7).

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh local-auth            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh local-auth
```

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
