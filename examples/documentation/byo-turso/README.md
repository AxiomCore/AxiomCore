# byo-turso (E7)

Application tables in the customer's own Turso database, Axiom-managed auth in a separate store, one migration history, disconnect.

Status: lead row. Project `axq-w8x-byo`, wave 2 (examples matrix 3.1).
Depends on: W8.3 (placement BYO, merged as C1); O3 second Turso account (G3).

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh byo-turso            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh byo-turso
```

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
