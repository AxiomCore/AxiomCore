"""E13 local checks: the verifier and the FastAPI dependency, with a synthetic issuer key."""
import base64
import hashlib
import hmac
import json
import os
import time
import unittest

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

ISSUER = "https://auth.axiomcore.dev/11111111-1111-4111-8111-111111111111/22222222-2222-4222-8222-222222222222"
AUDIENCE = ISSUER + "/api"
os.environ["AXIOM_ISSUER"], os.environ["AXIOM_AUDIENCE"] = ISSUER, AUDIENCE

import axiom_verify  # noqa: E402
import main  # noqa: E402

KEY = Ed25519PrivateKey.generate()
OTHER = Ed25519PrivateKey.generate()
b64 = lambda raw: base64.urlsafe_b64encode(raw).rstrip(b"=").decode()
PUBLIC = KEY.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
JWKS = {"keys": [{"kty": "OKP", "crv": "Ed25519", "kid": "k1", "use": "sig", "alg": "EdDSA", "x": b64(PUBLIC)}]}
fetches = []


def fetch(issuer):
    fetches.append(issuer)
    return JWKS


def token(claims=None, header=None, key=KEY, alg_override=None):
    now = int(time.time())
    body = {"iss": ISSUER, "aud": AUDIENCE, "sub": "user-1", "sid": "s1", "iat": now, "nbf": now, "exp": now + 900,
            "token_use": "access", "amr": ["pwd"], "roles": ["member"], "permissions": [], **(claims or {})}
    body = {k: v for k, v in body.items() if v is not None}
    head = {"alg": "EdDSA", "typ": "at+jwt", "kid": "k1", **(header or {})}
    signing = f"{b64(json.dumps(head).encode())}.{b64(json.dumps(body).encode())}"
    if alg_override == "none":
        return signing + "."
    if alg_override == "HS256":
        return signing + "." + b64(hmac.new(PUBLIC, signing.encode(), hashlib.sha256).digest())
    return signing + "." + b64(key.sign(signing.encode()))


class VerifierTest(unittest.TestCase):
    def setUp(self):
        self.v = axiom_verify.AxiomVerifier(ISSUER, AUDIENCE, fetch=fetch)

    def refused(self, tok, reason):
        with self.assertRaises(axiom_verify.TokenRefused) as ctx:
            self.v.verify(tok)
        self.assertEqual(ctx.exception.reason, reason)

    def test_accepts_an_issuer_token(self):
        p = self.v.verify(token({"acr": "aal2", "amr": ["mfa", "pwd", "totp"], "tenant": "org_1"}))
        self.assertEqual((p.subject, p.tenant, p.acr), ("user-1", "org_1", "aal2"))
        self.assertEqual(self.v.verify(token()).acr, "aal1")

    def test_refusals(self):
        self.refused(token({"iss": ISSUER + "x"}), "issuer")
        self.refused(token({"iss": "https://auth.axiomcore.dev/other/env"}), "issuer")
        self.refused(token({"aud": ISSUER + "/other"}), "audience")
        self.refused(token({"exp": int(time.time()) - 120}), "expired")
        self.refused(token({"nbf": int(time.time()) + 600}), "not_yet_valid")
        self.refused(token(alg_override="none", header={"alg": "none"}), "algorithm")
        self.refused(token(alg_override="HS256", header={"alg": "HS256"}), "algorithm")
        self.refused(token(header={"typ": "JWT"}), "algorithm")
        self.refused(token(key=OTHER), "signature")
        self.refused(token(header={"kid": "k9"}), "unknown_key")
        self.refused(token({"token_use": "id"}), "token_use")
        self.refused(token({"client_id": "axcl_" + "a" * 22}), "oauth_client_not_allowed")
        self.refused(token({"scope": "openid"}), "oauth_client_not_allowed")
        self.refused("a.b", "malformed")
        tampered = token().split(".")
        tampered[1] = b64(json.dumps({"iss": ISSUER, "aud": AUDIENCE, "sub": "admin", "exp": 9999999999, "token_use": "access"}).encode())
        self.refused(".".join(tampered), "signature")

    def test_jwks_is_cached(self):
        fetches.clear()
        for _ in range(5):
            self.v.verify(token())
        self.assertEqual(len(fetches), 1)


class ApiTest(unittest.TestCase):
    def setUp(self):
        from fastapi.testclient import TestClient
        main.verifier = axiom_verify.AxiomVerifier(ISSUER, AUDIENCE, fetch=fetch)
        self.client = TestClient(main.app)

    def test_me(self):
        self.assertEqual(self.client.get("/me").status_code, 401)
        bad = self.client.get("/me", headers={"Authorization": "Bearer " + token({"aud": "x"})})
        self.assertEqual(bad.status_code, 401)
        self.assertIn('error_description="audience"', bad.headers["www-authenticate"])
        ok = self.client.get("/me", headers={"Authorization": "Bearer " + token({"roles": ["admin"]})})
        self.assertEqual(ok.status_code, 200)
        self.assertEqual(ok.json(), {"subject": "user-1", "tenant": None, "roles": ["admin"], "acr": "aal1"})


if __name__ == "__main__":
    unittest.main()
