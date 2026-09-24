set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments := true

python := env_var_or_default("AXIOM_RELEASE_PYTHON", "python3.12")

default:
    @just release

# One operator interface; run `just release help` for the current commands.
release action="status" component="" version="":
    {{ python }} -B release/control/release_cli.py "$1" "$2" "$3"
