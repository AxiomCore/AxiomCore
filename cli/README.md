# Axiom CLI

CLI 0.148.0 provides one offline contract interface:

```sh
axiom check frontend/main.acore
axiom check backend/axiom.acore --json
axiom check database/schema.acore
axiom build frontend/main.acore       # frontend/frontend.axiom
axiom build backend/axiom.acore       # backend/backend.axiom
axiom build database/schema.acore     # database/database.axiom
axiom build database/schema.acore --out dist/schema.axiom
axiom inspect database/database.axiom
```

The nearest `AxiomDeps.toml` selects the compiler. Legacy profile/module headers
remain supported. Default artifacts are beside the input; explicit output paths
are relative to the current directory. Check returns diagnostics and a non-zero
status for invalid source, without changing project outputs. Frontend snapshots
are compiled UI IR; `axiom ui build` still creates runnable `.axiomapp` bundles.

Backend-only options: `--variant`, `--compatibility-baseline`, `--release`.
Frontend-only options: `--target web|ios|android`, `--lock FILE`.

Install the bundled language server without a download or source checkout:

```sh
axiom install lsp-server
~/.axiom/bin/acore-lsp --version-json
axiom lsp
axiom lsp --version-json
```

Windows installs `acore-lsp.exe`. Use the printed absolute path in VS Code User
`axiom.server.path` or Zed `lsp.acore-lsp.binary.path`. Alternatively configure the
editor to launch the absolute CLI path with `lsp`. Re-run the installer after CLI
updates. Apple Silicon macOS is validated; other host acceptance remains pending.

Build this sibling workspace with `cargo build --locked --manifest-path
cli/Cargo.toml` from the AxiomCore repository. The local `scripts/laxiom` wrapper
selects the newest debug/release executable, or `AXIOM_LOCAL_CLI_BIN` when set.
It uses a loopback control plane for Cloud commands; public `axiom` uses its normal
Cloud configuration. Install the wrapper with `scripts/install-laxiom`.

See the [public CLI reference](../docs/content/docs/tooling/cli-reference.mdx) for
all command families, editor settings, Cloud project/release steps, and migration
from the former `axiom.axiom` default. Run `axiom --help` or `axiom COMMAND --help`
for the exact installed command tree.
