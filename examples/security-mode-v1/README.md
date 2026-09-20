# Security analysis fixture

This directory contains a safe audit-mode baseline and an intentionally unsafe
strict contract.

## Safe baseline

```bash
cd examples/security-mode-v1
axiom security check axiom.acore
axiom build axiom.acore
```

The analysis should complete without error findings. The compiled artifact
contains the normalized security manifest and source-evidence summary used by
later review tooling.

## Expected strict rejection

```bash
axiom security check axiom.unsafe.acore
```

This command must fail with `AXSEC-001`. The unsafe contract declares a route
without an explicit public declaration or authorization guard while strict
mode is enabled.

The failing fixture is test input, not a configuration to copy into an
application.
