# Local validation and release

Builds, checks and artifact uploads are owned by the local release cycle.
GitHub Actions is reserved for SDK publication; push, pull-request, scheduled
and release-tag CI jobs are disabled.

Run release operations from the sibling private `Axiom-release-plane` checkout:

```sh
just release containers
just release web
```

Useful repository checks:

```sh
python3 scripts/check-public-boundary.py
# CLI, docs and server acceptance run through the local release controller.
```

`former-github-actions/` preserves the previous command recipes for reference;
these files are outside GitHub's workflow directory and cannot start Actions.
Setup steps and GitHub expressions in those recipes are historical, not a
local execution interface. Use the commands above and the controller's operator guide. Native/editor acceptance on this Mac covers macOS ARM64;
Windows and other host-platform qualification must run on those platforms.
Upstream Lynx qualification remains available through the private controller's
qualification scripts when evaluating a new engine version.

The SDK publication workflow retained under `.github/workflows/` keeps its
existing repository identity, protected publication environment and OIDC trust.


## Database developer workflow acceptance

`database-dx-acceptance.py` executes the installed CLI against isolated SQLite files and a real terminal. It covers source/artifact run, version prompts/CI preparation, frozen multi-target promotion, history/comparison, drift/trust rejection, populated migration/rollback, exact adoption, concurrent apply, source journal recovery and the native SQL console. It never changes an existing project/database:

```sh
python3 validation/database-dx-acceptance.py --cli /path/to/axiom --report /path/to/acceptance.json
```

`database-dx.Dockerfile` packages a Linux native qualification environment with PostgreSQL 17/18. Run `database-dx-linux.py` after compiling the CLI with the maintainer-only `axiom-database/test-faults` feature to exercise real commit interruption. Standard builds do not expose crash controls. Artifact bundling is performed through `cli/scripts/bundle-database-runtime.py`, with qualified PostgreSQL prefixes and an optional matching web-host cache. These are maintainer tools; developers running the example do not invoke PostgreSQL tools or write JSON bindings.

### Database endpoint-handler acceptance

`database-dx-fullstack.py` also checks native update/delete, repeated deletion, browser deletion and persistence after reload on both supported engines. `database-handlers-acceptance.py` copies the three playground database projects to disposable directories, supplies disposable targets and seeds, and proves their downloaded handlers run natively with a working frontend and hidden generated state:

```sh
python3 validation/database-handlers-acceptance.py --cli /absolute/path/to/axiom --report /temporary/path/handlers.json
```

Maintainer PostgreSQL provisioning uses the qualified managed validator assets. Developers follow each example README and use Axiom commands with their connection URL. Neither suite targets user databases.
