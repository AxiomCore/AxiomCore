"""A minimal OAuth application (relying party) for the Axiom OIDC issuer (examples matrix E11).

Authorization code with PKCE (S256), state and nonce; the ID token is verified locally
(EdDSA or RS256 from the issuer JWKS, iss, aud, nonce, exp). Prints claim names only.
"""
from __future__ import annotations

import base64
import hashlib
import json
import secrets
import time
import urllib.parse


def b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


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
