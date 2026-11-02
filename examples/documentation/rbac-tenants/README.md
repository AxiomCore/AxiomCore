# rbac-tenants (E5)

Roles, permissions, organizations, tenant selection, Permission/Role/Ownership guards, auth/policy.json with axiom auth policy check in CI, narrowing refused by the runtime.

Status: lead row. Project `axq-w8x-rbac (libSQL) / axq-w8x-pg (PostgreSQL)`, wave 2 / P1 (examples matrix 3.1).
Depends on: W8.6 (policy, authz_gen; CLI `axiom auth policy` from lane D2a); G7 for PostgreSQL.

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh rbac-tenants            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh rbac-tenants
```

A policy release is signed with the project's enrolled database release key: pass
`--schema <database contract>` (the contract's own `AxiomDeps.toml` beside it; `check.sh` takes
`AXIOM_W8_SCHEMA`) or set `AXIOM_DATABASE_SIGNING_KEY`.

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
