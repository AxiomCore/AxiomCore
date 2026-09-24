set shell := ["bash", "-euo", "pipefail", "-c"]
python := env_var_or_default("AXIOM_RELEASE_PYTHON", "python3.12")

# One entry point for planning and validating AxiomCore platform releases.
# Component build implementations remain in their owning repositories.
default:
  @just --list

release-test:
  {{python}} -B -m unittest discover -s release/control -p 'test_*.py' -v

# Candidate versions for every catalog component; published identities require
# a separately authenticated release baseline.
release-versions:
  {{python}} -B release/control/versions.py show

release-version-set component version:
  {{python}} -B release/control/versions.py set "{{component}}" "{{version}}"

release-version-replace component version:
  {{python}} -B release/control/versions.py set "{{component}}" "{{version}}" --replace

# Standard evidence paths are derived from release/control/intent.json and
# the mounted release volume. No caller-supplied absolute path is needed.
release-train-plan:
  {{python}} -B release/control/train.py plan

release-train-status:
  {{python}} -B release/control/train.py status

release-plan:
  {{python}} -B release/control/ctl.py plan

release-plan-save out:
  {{python}} -B release/control/ctl.py plan --out "{{out}}"

# CI and release gates use strict mode so dirty declared sources fail closed.
release-plan-strict out:
  {{python}} -B release/control/ctl.py plan --strict --out "{{out}}"

release-plan-against baseline:
  {{python}} -B release/control/ctl.py plan --baseline "{{baseline}}"

release-plan-against-save baseline out:
  {{python}} -B release/control/ctl.py plan --baseline "{{baseline}}" --out "{{out}}"

release-plan-against-strict baseline out:
  {{python}} -B release/control/ctl.py plan --baseline "{{baseline}}" --strict --out "{{out}}"

release-plan-force baseline component out:
  {{python}} -B release/control/ctl.py plan --baseline "{{baseline}}" --force "{{component}}" --strict --out "{{out}}"

release-plan-component component:
  {{python}} -B release/control/ctl.py plan --only "{{component}}"

release-plan-component-save component out:
  {{python}} -B release/control/ctl.py plan --only "{{component}}" --strict --out "{{out}}"

release-plan-component-against component baseline out:
  {{python}} -B release/control/ctl.py plan --only "{{component}}" --baseline "{{baseline}}" --strict --out "{{out}}"

release-mount:
  {{python}} -B release/control/ctl.py mount

release-preflight:
  {{python}} -B release/control/ctl.py preflight

# Build one selected component from a saved plan; never publishes it.
release-build component plan:
  {{python}} -B release/control/ctl.py build --component "{{component}}" --plan "{{plan}}"

release-verify receipt:
  {{python}} -B release/control/ctl.py verify --receipt "{{receipt}}"

release-receipt component plan artifact out:
  {{python}} -B release/control/ctl.py receipt --component "{{component}}" --plan "{{plan}}" --artifact "{{artifact}}" --out "{{out}}"

release-stage-one plan train receipt out:
  {{python}} -B release/control/ctl.py stage --plan "{{plan}}" --train-id "{{train}}" --receipt "{{receipt}}" --out "{{out}}"

release-stage-intent-one plan train receipt out:
  {{python}} -B release/control/ctl.py stage --intent release/control/intent.json --plan "{{plan}}" --train-id "{{train}}" --receipt "{{receipt}}" --out "{{out}}"

release-notes plan:
  {{python}} -B release/control/ctl.py notes --plan "{{plan}}" --enforce

release-notes-save plan out:
  {{python}} -B release/control/ctl.py notes --plan "{{plan}}" --enforce --out "{{out}}"

release-notes-owner-save owner plan out:
  {{python}} -B release/control/ctl.py notes --plan "{{plan}}" --owner "{{owner}}" --enforce --out "{{out}}"

# Preview or explicitly apply version bumps and owned release-note fragments.
# Apply saves originals on the external release volume; it never commits.
release-prepare:
  {{python}} -B release/control/flow.py

release-prepare-save out:
  {{python}} -B release/control/flow.py --out "{{out}}"

release-prepare-apply:
  {{python}} -B release/control/flow.py --apply

# A passing gate is ready for a publisher, not proof that anything is live.
release-gate plan:
  {{python}} -B release/control/gate.py --plan "{{plan}}"

release-gate-staged plan stage receipt out:
  {{python}} -B release/control/gate.py --plan "{{plan}}" --stage "{{stage}}" --receipt "{{receipt}}" --out "{{out}}"
