# hosted-persistence (E3/E4)

One Acore web plus backend app on an Axiom-managed database: hosted migrate, runtime-data through the gateway with a Secrets lease, guarded writes.

Status: lead row. Project `axq-w8x-hosted (E3) / axq-w8x-pg (E4)`, wave 1 / P1 (examples matrix 3.1).
Depends on: E3: none. E4: G7 (pool B slot, Stripe test allowlist).

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh hosted-persistence            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh hosted-persistence
```

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
