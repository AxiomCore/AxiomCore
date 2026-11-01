"""axiom-auth-config/v1 export and diff checks (examples matrix section 7)."""
import json
import sys

SECRET_KEYS = {"secret", "client_secret", "password", "token", "auth_token", "api_key_secret", "turnstile_secret", "private_key"}


def walk(value, path=""):
    if isinstance(value, dict):
        for k, v in value.items():
            if k in SECRET_KEYS and not (isinstance(v, dict) and set(v) == {"ref"}):
                raise SystemExit(f"secret value at {path}/{k}")
            walk(v, f"{path}/{k}")
    elif isinstance(value, list):
        for i, v in enumerate(value):
            walk(v, f"{path}/{i}")


if sys.argv[1] == "--diff":
    diff = json.load(open(sys.argv[2]))
    header = diff.get("header", diff)
    for k in ("environment", "project", "runtime_target", "base_revision"):
        assert k in header, k
    for item in diff.get("items", []):
        assert item.get("impact") in {"widening", "narrowing", "credential", "neutral"}, item.get("id")
    walk(diff)
    print(f"diff names environment, runtime target and {len(diff.get('items', []))} impacts")
else:
    raw = open(sys.argv[1], "rb").read()
    assert len(raw) <= 256 * 1024
    doc = json.loads(raw)
    assert doc.get("format") == "axiom-auth-config/v1"
    assert doc.get("source", {}).get("kind") == "local"
    walk(doc)
    print("export holds secret references only")
