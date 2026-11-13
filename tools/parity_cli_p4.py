"""P4 cli-workspace caller anchors and exemptions for the parity checker.

Same rules as `parity_cli.CALLERS`: `evidence` is the production call site and
`command_evidence` its dispatch anchor; both must stay in the named source file.
Sub-group anchors live in `parity_cli_p4_<group>.py`.
"""
from parity_cli_p4_workspace import CALLERS as _WORKSPACE
from parity_cli_p4_sandbox import CALLERS as _SANDBOX
from parity_cli_p4_contracts import CALLERS as _CONTRACTS

CALLERS = _WORKSPACE + _SANDBOX + _CONTRACTS

_BROWSER_SESSION = (
    'Browser-session route of the dashboard sign-in (email code and HttpOnly session cookie). '
    'The CLI signs in with the device flow (`axiom login`, `axiom login --step-up`) and keeps '
    'its own refreshed session, so it never calls this route.'
)

EXEMPT = {
    ('POST', '/api/v1/auth/send'): _BROWSER_SESSION,
    ('POST', '/api/v1/auth/verify'): _BROWSER_SESSION,
    ('GET', '/api/v1/auth/session'): _BROWSER_SESSION,
    ('POST', '/api/v1/auth/session/refresh'): _BROWSER_SESSION,
    ('DELETE', '/api/v1/auth/session'): _BROWSER_SESSION,
}
