import base64
import hashlib
import json
import secrets
import unittest

import rp


def _prime(bits):
    while True:
        n = secrets.randbits(bits) | (1 << (bits - 1)) | 1
        if all(pow(a, n - 1, n) == 1 for a in (2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37)):
            return n


def _rsa_test_key():
    e = 65537
    while True:
        p, q = _prime(1024), _prime(1024)
        phi = (p - 1) * (q - 1)
        if p != q and phi % e:
            return p * q, e, pow(e, -1, phi)


def _rs256(n, d, kid, claims):
    b = lambda raw: base64.urlsafe_b64encode(raw).rstrip(b"=").decode()
    signing_input = b(json.dumps({"alg": "RS256", "kid": kid}).encode()) + "." + b(json.dumps(claims).encode())
    size = (n.bit_length() + 7) // 8
    t = rp._SHA256_DIGEST_INFO + hashlib.sha256(signing_input.encode()).digest()
    em = b"\x00\x01" + b"\xff" * (size - len(t) - 3) + b"\x00" + t
    return signing_input + "." + b(pow(int.from_bytes(em, "big"), d, n).to_bytes(size, "big"))


class RelyingPartyTest(unittest.TestCase):
    def test_pkce_and_url(self):
        url, tx = rp.authorize_url({"authorization_endpoint": "https://auth.axiomcore.dev/p/e/oauth2/authorize"}, "axcl_" + "a" * 22, "http://127.0.0.1:5178/callback")
        challenge = base64.urlsafe_b64encode(hashlib.sha256(tx["verifier"].encode()).digest()).rstrip(b"=").decode()
        self.assertIn("code_challenge=" + challenge, url)
        self.assertIn("code_challenge_method=S256", url)
        self.assertIn("nonce=" + tx["nonce"], url)

    def test_id_token_checks(self):
        ok = {"iss": "https://i", "aud": "c", "nonce": "n", "exp": 2000}
        rp.check_id_token(ok, issuer="https://i", client_id="c", nonce="n", now=1000)
        for bad, field in [({"iss": "https://x"}, "iss"), ({"aud": "d"}, "aud"), ({"nonce": "m"}, "nonce"), ({"exp": 999}, "exp")]:
            with self.assertRaisesRegex(ValueError, field):
                rp.check_id_token({**ok, **bad}, issuer="https://i", client_id="c", nonce="n", now=1000)

    def test_rs256_signature(self):
        n, e, d = _rsa_test_key()
        to_b64 = lambda i: base64.urlsafe_b64encode(i.to_bytes((i.bit_length() + 7) // 8, "big")).rstrip(b"=").decode()
        jwks = {"keys": [{"kty": "RSA", "kid": "k1", "use": "sig", "n": to_b64(n), "e": to_b64(e)}]}
        token = _rs256(n, d, "k1", {"iss": "https://i", "sub": "u"})
        self.assertEqual(rp.verify_jwt(token, jwks)["sub"], "u")
        head, body, sig = token.split(".")
        forged_body = base64.urlsafe_b64encode(json.dumps({"iss": "https://i", "sub": "v"}).encode()).rstrip(b"=").decode()
        for bad in (head + "." + forged_body + "." + sig, _rs256(n, d, "other", {"sub": "u"}), head + "." + body + "." + sig[::-1]):
            with self.assertRaisesRegex(ValueError, "signature"):
                rp.verify_jwt(bad, jwks)
        none = base64.urlsafe_b64encode(b'{"alg":"none","kid":"k1"}').rstrip(b"=").decode() + "." + body + "."
        with self.assertRaisesRegex(ValueError, "algorithm"):
            rp.verify_jwt(none, jwks)


if __name__ == "__main__":
    unittest.main()
