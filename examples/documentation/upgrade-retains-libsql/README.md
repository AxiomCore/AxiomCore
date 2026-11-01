# upgrade-retains-libsql (E8)

Free to Pro upgrade keeps the libSQL resource and endpoint with the Free storage and request profile; same auth API.

Status: lead row. Project `axq-w8x-upgrade`, wave 3 (examples matrix 3.1).
Depends on: Stripe test mode (G7 allowlist).

Run (lead, from the packaged archive with the installed release candidate):

```sh
bash ../../../tools/w8D2/run-row.sh upgrade-retains-libsql            # local, offline steps only
AXIOM_W8_APPLY=yes AXIOM_W8_PROJECT=<slug> bash ../../../tools/w8D2/run-row.sh upgrade-retains-libsql
```

`check.sh` prints `PASS`, `FAIL` and `LEAD` lines; `LEAD` lines are the manual or browser
steps of section 4 "Assertions per row". Secrets come only from files under
`secrets-w8x/` (mode 0600), stdin or prompts, never argv. Cleanup follows matrix 3.1:
branches, then resources, then providers and test users, then Stripe test subscriptions,
then the project.
