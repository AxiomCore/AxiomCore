"""E12 local flows against `axiom auth emulate` (examples matrix section 7).

Usage: python3 -B flow.py <password|oidc> <state.json> <work dir>

Standard library only. Talks to loopback URLs only (the emulator's state file names them).
Tokens and the synthetic password go to 0600 files in the work directory, never to output.
"""
import base64
import hashlib
import json
import os
import re
import secrets
import sys
import urllib.error
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect)


def loopback(url):
    host = urllib.parse.urlsplit(url).hostname
    assert host in ("127.0.0.1", "localhost"), f"not a loopback URL: {host}"
    return url


def call(method, url, body=None, bearer=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(loopback(url), data=data, method=method)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    if bearer:
        req.add_header("Authorization", "Bearer " + bearer)
    try:
        with OPENER.open(req, timeout=30) as r:
            return r.status, r.headers, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


def ok(method, url, body=None, bearer=None):
    status, _, raw = call(method, url, body, bearer)
    assert status == 200, f"{method} {urllib.parse.urlsplit(url).path}: {status}"
    return json.loads(raw)


def private(work, name, value):
    path = os.path.join(work, name)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(value)


def mail_code(state, challenge):
    mail = ok("GET", state["mail"] + ".json")["mail"]
    message = next(m for m in mail if m["challenge_id"] == challenge)
    code = re.search(r"\b(\d{6})\b", message["body_text"]).group(1)
    status, _, page = call("GET", state["mail"])
    assert status == 200 and code.encode() in page, "the mail viewer shows the code"
    return code


def password(state, work):
    issuer = state["issuer"]
    secret = "Synthetic-" + secrets.token_urlsafe(18)
    private(work, "password", secret)
    email = "ada@example.com"
    r = ok("POST", issuer + "/v1/signup", {"email": email, "password": secret})
    code = mail_code(state, r["challenge_id"])
    status, _, _ = call("POST", issuer + "/v1/signup/verify", {"challenge_id": r["challenge_id"], "code": "999999" if code != "999999" else "000000"})
    assert status == 400, "a wrong code is refused"
    t = ok("POST", issuer + "/v1/signup/verify", {"challenge_id": r["challenge_id"], "code": code})
    assert t["user"]["email"] == email
    print("sign-up verified with the code from the mail viewer")
    status, _, _ = call("POST", issuer + "/v1/sign-in/password", {"email": email, "password": secret + "x"})
    assert status == 401, "a wrong password is refused"
    p = ok("POST", issuer + "/v1/sign-in/password", {"email": email, "password": secret})
    rotated = ok("POST", issuer + "/v1/token/refresh", {"refresh_token": p["refresh_token"]})
    assert rotated["refresh_token"] != p["refresh_token"]
    status, _, _ = call("POST", issuer + "/v1/token/refresh", {"refresh_token": p["refresh_token"]})
    assert status == 401, "the rotated refresh token is refused"
    status, _, _ = call("POST", issuer + "/v1/token/refresh", {"refresh_token": rotated["refresh_token"]})
    assert status == 401, "reuse ended the session"
    print("password sign-in; refresh rotates; reuse of a rotated token ends the session")
    c = ok("POST", issuer + "/v1/email/code", {"email": email})
    e = ok("POST", issuer + "/v1/email/code/verify", {"challenge_id": c["challenge_id"], "code": mail_code(state, c["challenge_id"])})
    me = ok("GET", issuer + "/v1/user", bearer=e["access_token"])
    assert me["email_verified"] is True and me["has_password"] is True
    private(work, "access-token", e["access_token"])
    private(work, "refresh-token", e["refresh_token"])
    print("email-code sign-in; access token saved privately for the backend call")


def oidc(state, work):
    issuer = state["issuer"]
    verifier = secrets.token_urlsafe(48)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    app = "http://127.0.0.1:5173/auth/callback"
    query = urllib.parse.urlencode({"redirect_uri": app, "state": "e12", "code_challenge": challenge, "code_challenge_method": "S256", "login_hint": "jo@example.com"})
    status, headers, _ = call("GET", f"{issuer}/v1/providers/local/start?{query}")
    assert status == 302
    authorize = headers["Location"]
    assert authorize.startswith(state["oidc"] + "/authorize?"), "the provider is the local mock issuer"
    status, _, _ = call("GET", authorize.replace("jo%40example.com", "eve%40example.com"))
    assert status == 400, "only synthetic identities sign in"
    status, headers, _ = call("GET", authorize)
    assert status == 302
    status, headers, _ = call("GET", headers["Location"])
    assert status == 302
    back = urllib.parse.urlsplit(headers["Location"])
    q = dict(urllib.parse.parse_qsl(back.query))
    assert headers["Location"].startswith(app) and q["state"] == "e12" and q["iss"] == issuer
    t = ok("POST", issuer + "/v1/token/authorization-code", {"code": q["code"], "code_verifier": verifier, "redirect_uri": app})
    assert t["user"]["email"] == "jo@example.com"
    discovery = ok("GET", state["oidc"] + "/.well-known/openid-configuration")
    assert discovery["issuer"] == state["oidc"] and discovery["axiom_synthetic_identities_only"] is True
    print("mock OIDC sign-in of the synthetic identity jo@example.com")


if __name__ == "__main__":
    command, state_file, work = sys.argv[1:4]
    state = json.load(open(state_file))
    for key in ("issuer", "mail", "oidc"):
        loopback(state[key])
    {"password": password, "oidc": oidc}[command](state, work)
