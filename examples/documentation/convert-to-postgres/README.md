# convert-to-postgres (E9)

Explicit libSQL to PostgreSQL conversion with report, receipt and cutover; Pro only (OQ-11).

Status: lead row. Project `axq-w8x-upgrade`, wave 3 (examples matrix 3.1).
Depends on: W8.4 (D3, C2); G7; runs after E8 on the same project.

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh convert-to-postgres            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh convert-to-postgres
```

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
