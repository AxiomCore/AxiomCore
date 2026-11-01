import base64
import hashlib
import unittest

import rp


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


if __name__ == "__main__":
    unittest.main()
