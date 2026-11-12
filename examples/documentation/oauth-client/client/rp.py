"""A minimal OAuth application (relying party) for the Axiom OIDC issuer (examples matrix E11).

Authorization code with PKCE (S256), state and nonce; the ID token is verified locally: the
signature against the issuer JWKS (RS256 with the standard library, EdDSA when the
`cryptography` package is installed), then iss, aud, nonce and exp. Prints claim names and
check results only; tokens go to 0600 files under --out when it is given, never to output.

  python3 rp.py --issuer https://auth.axiomcore.dev/<project>/<environment> --client-id axcl_...
      [--port 5178] [--scope "openid email"] [--out DIR] [--url-file FILE] [--no-open] [--timeout 300]

Register the client with redirect URI http://127.0.0.1:<port>/callback (public, PKCE).
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import http.server
import json
import os
import secrets
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import webbrowser


def b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def b64url_decode(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def pkce() -> tuple[str, str]:
    verifier = b64url(secrets.token_bytes(32))
    return verifier, b64url(hashlib.sha256(verifier.encode()).digest())


def authorize_url(discovery: dict, client_id: str, redirect_uri: str, scope: str = "openid email"):
    verifier, challenge = pkce()
    state, nonce = b64url(secrets.token_bytes(16)), b64url(secrets.token_bytes(16))
    query = {"response_type": "code", "client_id": client_id, "redirect_uri": redirect_uri, "scope": scope,
             "state": state, "nonce": nonce, "code_challenge": challenge, "code_challenge_method": "S256"}
    return discovery["authorization_endpoint"] + "?" + urllib.parse.urlencode(query), {"verifier": verifier, "state": state, "nonce": nonce}


def check_id_token(claims: dict, *, issuer: str, client_id: str, nonce: str, now: float | None = None) -> None:
    now = time.time() if now is None else now
    if claims.get("iss") != issuer:
        raise ValueError("iss")
    aud = claims.get("aud")
    if not (aud == client_id or (isinstance(aud, list) and client_id in aud)):
        raise ValueError("aud")
    if claims.get("nonce") != nonce:
        raise ValueError("nonce")
    if not isinstance(claims.get("exp"), int) or claims["exp"] <= now:
        raise ValueError("exp")


# SHA-256 DigestInfo prefix of an RSASSA-PKCS1-v1_5 signature (RFC 8017 9.2).
_SHA256_DIGEST_INFO = bytes.fromhex("3031300d060960864801650304020105000420")


def _rs256_verify(jwk: dict, signing_input: bytes, signature: bytes) -> bool:
    n = int.from_bytes(b64url_decode(jwk["n"]), "big")
    e = int.from_bytes(b64url_decode(jwk["e"]), "big")
    size = (n.bit_length() + 7) // 8
    if size < 256 or len(signature) != size:
        return False
    em = pow(int.from_bytes(signature, "big"), e, n).to_bytes(size, "big")
    t = _SHA256_DIGEST_INFO + hashlib.sha256(signing_input).digest()
    expected = b"\x00\x01" + b"\xff" * (size - len(t) - 3) + b"\x00" + t
    return secrets.compare_digest(em, expected)


def _eddsa_verify(jwk: dict, signing_input: bytes, signature: bytes) -> bool:
    try:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    except ImportError as missing:
        raise ValueError("signature: EdDSA needs the cryptography package") from missing
    try:
        Ed25519PublicKey.from_public_bytes(b64url_decode(jwk["x"])).verify(signature, signing_input)
        return True
    except InvalidSignature:
        return False


def verify_jwt(token: str, jwks: dict, algorithms: tuple[str, ...] = ("RS256", "EdDSA")) -> dict:
    """Returns the claims of a JWT whose signature verifies with a key of `jwks`; raises ValueError."""
    try:
        header_b64, payload_b64, signature_b64 = token.split(".")
        header = json.loads(b64url_decode(header_b64))
        claims = json.loads(b64url_decode(payload_b64))
        signature = b64url_decode(signature_b64)
    except (ValueError, json.JSONDecodeError) as bad:
        raise ValueError("signature: malformed token") from bad
    alg = header.get("alg")
    if alg not in algorithms:
        raise ValueError("signature: algorithm not accepted")
    keys = [k for k in jwks.get("keys", []) if k.get("kid") == header.get("kid") and k.get("use", "sig") == "sig"]
    if len(keys) != 1:
        raise ValueError("signature: no unique key for kid")
    key, signing_input = keys[0], (header_b64 + "." + payload_b64).encode()
    if alg == "RS256" and key.get("kty") == "RSA":
        ok = _rs256_verify(key, signing_input, signature)
    elif alg == "EdDSA" and key.get("kty") == "OKP" and key.get("crv") == "Ed25519":
        ok = _eddsa_verify(key, signing_input, signature)
    else:
        raise ValueError("signature: key type does not match the algorithm")
    if not ok:
        raise ValueError("signature")
    return claims


def _get_json(url: str, token: str | None = None) -> dict:
    headers = {"Accept": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=20) as reply:
        return json.loads(reply.read())


def _get_status(url: str, token: str) -> tuple[int, dict]:
    try:
        return 200, _get_json(url, token)
    except urllib.error.HTTPError as failed:
        return failed.code, {}


def _post_form(url: str, form: dict) -> tuple[int, dict]:
    request = urllib.request.Request(url, data=urllib.parse.urlencode(form).encode(), method="POST",
                                     headers={"Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=20) as reply:
            return reply.status, json.loads(reply.read())
    except urllib.error.HTTPError as failed:
        try:
            return failed.code, json.loads(failed.read())
        except json.JSONDecodeError:
            return failed.code, {}


def _private(out: str | None, name: str, value: str) -> None:
    if not out:
        return
    os.makedirs(out, mode=0o700, exist_ok=True)
    fd = os.open(os.path.join(out, name), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(value)


def _wait_for_callback(port: int, timeout: float) -> dict:
    result: dict = {}
    done = threading.Event()

    class Callback(http.server.BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802 (http.server API)
            url = urllib.parse.urlsplit(self.path)
            if url.path != "/callback":
                self.send_error(404)
                return
            result.update({k: v[0] for k, v in urllib.parse.parse_qs(url.query).items()})
            body = b"<!doctype html><title>Signed in</title><p>You can close this tab.</p>"
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            done.set()

        def log_message(self, *args):  # the query holds the code: never log it
            pass

    server = http.server.HTTPServer(("127.0.0.1", port), Callback)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        if not done.wait(timeout):
            raise SystemExit("FAIL no callback before the timeout")
    finally:
        server.shutdown()
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Sign in through the Axiom OIDC issuer (examples matrix E11).")
    parser.add_argument("--issuer", required=True)
    parser.add_argument("--client-id", required=True)
    parser.add_argument("--port", type=int, default=5178)
    parser.add_argument("--scope", default="openid email")
    parser.add_argument("--out", help="private directory for the tokens (0600 files)")
    parser.add_argument("--url-file", help="also write the authorization URL here (for a scripted browser)")
    parser.add_argument("--no-open", action="store_true", help="do not open the system browser")
    parser.add_argument("--timeout", type=float, default=300)
    args = parser.parse_args(argv)
    issuer = args.issuer.rstrip("/")
    redirect_uri = "http://127.0.0.1:%d/callback" % args.port
    checks: dict[str, bool] = {}

    discovery = _get_json(issuer + "/.well-known/openid-configuration")
    checks["discovery issuer"] = discovery.get("issuer") == issuer
    checks["discovery S256"] = "S256" in discovery.get("code_challenge_methods_supported", [])
    jwks = _get_json(discovery["jwks_uri"])
    url, tx = authorize_url(discovery, args.client_id, redirect_uri, args.scope)
    if args.url_file:
        _private(os.path.dirname(os.path.abspath(args.url_file)), os.path.basename(args.url_file), url)
    print("open the authorization URL to sign in" + ("" if args.no_open else " (opening the browser)"), flush=True)
    if not args.no_open:
        webbrowser.open(url)

    callback = _wait_for_callback(args.port, args.timeout)
    checks["callback state"] = secrets.compare_digest(callback.get("state", ""), tx["state"])
    if "iss" in callback:
        checks["callback iss"] = callback["iss"] == issuer
    if "error" in callback or "code" not in callback:
        print("authorization error:", callback.get("error", "no code"))
        return 1

    status, tokens = _post_form(discovery["token_endpoint"], {
        "grant_type": "authorization_code", "code": callback["code"], "redirect_uri": redirect_uri,
        "client_id": args.client_id, "code_verifier": tx["verifier"]})
    checks["code exchange"] = status == 200 and bool(tokens.get("id_token")) and bool(tokens.get("access_token"))
    if not checks["code exchange"]:
        print("token endpoint:", status, tokens.get("error"))
        return 1

    algs = tuple(discovery.get("id_token_signing_alg_values_supported") or ("RS256",))
    try:
        claims = verify_jwt(tokens["id_token"], jwks, algs)
        checks["ID token signature"] = True
        check_id_token(claims, issuer=issuer, client_id=args.client_id, nonce=tx["nonce"])
        checks["ID token iss, aud, nonce, exp"] = True
    except ValueError as failed:
        claims = {}
        checks["ID token " + str(failed)] = False
    tampered = tokens["id_token"][:-4] + ("AAAA" if not tokens["id_token"].endswith("AAAA") else "BBBB")
    try:
        verify_jwt(tampered, jwks, algs)
        checks["tampered ID token refused"] = False
    except ValueError:
        checks["tampered ID token refused"] = True

    if discovery.get("userinfo_endpoint"):
        status, info = _get_status(discovery["userinfo_endpoint"], tokens["access_token"])
        checks["userinfo sub matches the ID token"] = status == 200 and bool(claims) and info.get("sub") == claims.get("sub")
    # A replayed code is refused and revokes what the code issued (RFC 6749 4.1.2).
    replay, replayed = _post_form(discovery["token_endpoint"], {
        "grant_type": "authorization_code", "code": callback["code"], "redirect_uri": redirect_uri,
        "client_id": args.client_id, "code_verifier": tx["verifier"]})
    checks["code replay refused"] = replay == 400 and replayed.get("error") == "invalid_grant"
    if discovery.get("userinfo_endpoint"):
        checks["access token revoked after the replay"] = _get_status(discovery["userinfo_endpoint"], tokens["access_token"])[0] == 401
    _private(args.out, "id-token", tokens["id_token"])
    _private(args.out, "access-token", tokens["access_token"])
    if tokens.get("refresh_token"):
        _private(args.out, "refresh-token", tokens["refresh_token"])

    print("ID token claims:", ", ".join(sorted(claims)))
    if discovery.get("end_session_endpoint"):
        print("logout: end_session_endpoint advertised")
    for name, ok in checks.items():
        print(("PASS " if ok else "FAIL ") + name)
    return 0 if all(checks.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
