set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments := true

default:
    @just release

# The release controller lives in the private sibling repository.
release action="status" component="" version="":
    @test -d ../Axiom-release-plane/.git || { echo "Clone private AxiomCore/Axiom-release-plane beside this repository first." >&2; exit 1; }
    @cd ../Axiom-release-plane && just release "$1" "$2" "$3"
