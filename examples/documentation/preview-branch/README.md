# preview-branch (E16)

Schema and sanitized branches, test users, captured mail, reviewed promotion of a migration and a policy release.

Status: lead row. Project `axq-w8x-branch-free / axq-w8x-branch-pro`, wave 2 / P2 (examples matrix 3.1).
Depends on: W8.6 (branches-v1, lane D1 B2-B5); `axiom project resources branches` from lane D2a; G4, G7 for PostgreSQL.

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh preview-branch            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh preview-branch
```

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
