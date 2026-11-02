# hosted-persistence (E3/E4)

One Acore web plus backend app on an Axiom-managed database: hosted migrate, runtime-data through the gateway with a Secrets lease, guarded writes.

Status: lead row. Project `axq-w8x-hosted (E3) / axq-w8x-pg (E4)`, wave 1 / P1 (examples matrix 3.1).
Depends on: E3: none. E4: G7 (pool B slot, Stripe test allowlist).

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh hosted-persistence            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh hosted-persistence
```

The backend stores notes in the hosted database through a `connectDatabase` secret:
`backend/hosted.acore` (libSQL, E3) and `backend/hosted.postgres.acore` (PostgreSQL, E4) declare the
resource and its storage over the same actions and endpoints (`backend/application.acore`). On a
Linux container the runtime leases the credential with its service account through the data
gateway (`AxiomSecrets.toml`, source `axiom-cloud-database`), so no credential is stored in the
source, image or deployment. Rows are scoped to the verified token's subject (`sub`); schema
1.1.0 keeps them in `owned_notes`, keyed on (owner, id), so a note id is unique per user only and
another user's ids are never revealed by a duplicate refusal (1.0.0's `notes` stays, unused).
With `AXIOM_W8_PROJECT_ID`, `AXIOM_W8_ENVIRONMENT_ID` and `AXIOM_W8_RESOURCE_ID`, `check.sh` writes
that file and the deployment; with `AXIOM_W8_RUNTIME_SOURCE` and `AXIOM_W8_CONTEXT_OUT` it also
packages the `container-amd64` build context (`AXIOM_W8_ENGINE=postgresql` for E4).

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
