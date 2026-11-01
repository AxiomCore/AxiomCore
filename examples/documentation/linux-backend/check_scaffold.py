"""The deployment JSON holds the issuer and audience only: no key, secret, token or credential URL."""
import json
import re
import sys

d = json.load(open(sys.argv[1]))
auth = d.get("authentication", {})
assert auth.get("kind") == "axiomAuth", "axiomAuth provider"
assert set(auth) <= {"kind", "issuer", "audience", "oauthClients"}, sorted(auth)
text = json.dumps(d)
assert not re.search(r"(secret|password|token|privateKey|://[^/\s\"]+:[^@/\s\"]+@)", text, re.I), "credential-shaped value"
print("deployment holds no credential")
