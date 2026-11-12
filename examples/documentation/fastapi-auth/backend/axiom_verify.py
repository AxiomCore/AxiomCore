"""Verify Axiom access tokens in an existing Python backend (examples matrix E13).

Axiom access tokens are EdDSA (Ed25519) JWTs with `typ` `at+jwt`, signed by the
environment's issuer and published at `<issuer>/.well-known/jwks.json`. This module
checks the header, signature, issuer, audience, lifetime and token use, and refuses
OAuth application tokens (`client_id`, auth-advanced/v1 7.4) unless allowed.
"""
import base64
import json
import threading
import time
import urllib.request
from dataclasses import dataclass, field

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

JWKS_TTL_SECONDS = 300
CLOCK_SKEW_SECONDS = 30
MAX_TOKEN_BYTES = 8192


class TokenRefused(Exception):
    """The token is not acceptable; `reason` is a fixed code, never token content."""

    def __init__(self, reason: str):
        super().__init__(reason)
        self.reason = reason


def _b64url(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def fetch_jwks(issuer: str) -> dict:
    with urllib.request.urlopen(issuer + "/.well-known/jwks.json", timeout=5) as response:
        return json.loads(response.read(65536))


@dataclass
class Principal:
    subject: str
    session: str | None
    tenant: str | None
    roles: list[str]
    permissions: list[str]
    acr: str
    auth_time: int | None
    claims: dict = field(repr=False)


class AxiomVerifier:
    def __init__(self, issuer: str, audience: str, *, fetch=fetch_jwks, clock=time.time, allow_oauth_clients=False):
        if not issuer.startswith("https://") and not issuer.startswith("http://127.0.0.1"):
            raise ValueError("issuer must be https")
        self.issuer = issuer.rstrip("/")
        self.audience = audience
        self._fetch = fetch
        self._clock = clock
        self._allow_oauth = allow_oauth_clients
        self._keys: dict[str, Ed25519PublicKey] = {}
        self._fetched_at = 0.0
        self._lock = threading.Lock()

    def _key(self, kid: str) -> Ed25519PublicKey:
        with self._lock:
            now = self._clock()
            stale = now - self._fetched_at > JWKS_TTL_SECONDS
            # An unknown kid refetches at most once a minute (key rollover).
            if stale or (kid not in self._keys and now - self._fetched_at > 60):
                keys = {}
                for jwk in self._fetch(self.issuer).get("keys", []):
                    if jwk.get("kty") == "OKP" and jwk.get("crv") == "Ed25519" and isinstance(jwk.get("kid"), str) and jwk.get("use", "sig") == "sig":
                        keys[jwk["kid"]] = Ed25519PublicKey.from_public_bytes(_b64url(jwk["x"]))
                self._keys, self._fetched_at = keys, now
            key = self._keys.get(kid)
        if key is None:
            raise TokenRefused("unknown_key")
        return key

    def verify(self, token: str) -> Principal:
        if not isinstance(token, str) or len(token) > MAX_TOKEN_BYTES or token.count(".") != 2:
            raise TokenRefused("malformed")
        head_b64, body_b64, sig_b64 = token.split(".")
        try:
            header = json.loads(_b64url(head_b64))
            claims = json.loads(_b64url(body_b64))
            signature = _b64url(sig_b64)
        except (ValueError, json.JSONDecodeError):
            raise TokenRefused("malformed") from None
        if not isinstance(header, dict) or not isinstance(claims, dict):
            raise TokenRefused("malformed")
        # The algorithm is fixed: never taken from the token (no `none`, no HS256 key confusion).
        if header.get("alg") != "EdDSA" or header.get("typ") != "at+jwt" or not isinstance(header.get("kid"), str):
            raise TokenRefused("algorithm")
        try:
            self._key(header["kid"]).verify(signature, f"{head_b64}.{body_b64}".encode("ascii"))
        except InvalidSignature:
            raise TokenRefused("signature") from None
        now = self._clock()
        if claims.get("iss") != self.issuer:
            raise TokenRefused("issuer")
        aud = claims.get("aud")
        if not (aud == self.audience or (isinstance(aud, list) and self.audience in aud)):
            raise TokenRefused("audience")
        exp, nbf = claims.get("exp"), claims.get("nbf", claims.get("iat"))
        if not isinstance(exp, int) or exp <= now - CLOCK_SKEW_SECONDS:
            raise TokenRefused("expired")
        if isinstance(nbf, int) and nbf > now + CLOCK_SKEW_SECONDS:
            raise TokenRefused("not_yet_valid")
        if claims.get("token_use") != "access" or not isinstance(claims.get("sub"), str):
            raise TokenRefused("token_use")
        if "scope" in claims or ("client_id" in claims and not self._allow_oauth):
            raise TokenRefused("oauth_client_not_allowed")
        strings = lambda v: [x for x in v if isinstance(x, str)] if isinstance(v, list) else []
        acr = claims.get("acr") if claims.get("acr") in ("aal1", "aal2", "phr") else "aal1"
        return Principal(subject=claims["sub"], session=claims.get("sid"), tenant=claims.get("tenant"),
                         roles=strings(claims.get("roles")), permissions=strings(claims.get("permissions")),
                         acr=acr, auth_time=claims.get("auth_time") if isinstance(claims.get("auth_time"), int) else None,
                         claims=claims)
