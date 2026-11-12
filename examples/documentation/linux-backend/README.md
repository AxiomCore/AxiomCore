# linux-backend (E14)

Acore backend container on Linux (Cloud Run) with axiomAuth and runtime-data credentials injected by Secrets only; lease expiry and revocation.

Status: lead row. Project `axq-w8x-hosted / axq-w8x-pg`, wave 1 / P1 (examples matrix 3.1).
Depends on: G5 (Linux CLI on Cloud Build).

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh linux-backend            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh linux-backend
```

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
