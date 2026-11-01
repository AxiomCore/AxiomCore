import os
from typing import Annotated

from fastapi import Depends, FastAPI, Header, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel

from axiom_verify import AxiomVerifier, Principal, TokenRefused

# `axiom auth config pull` prints both; they are not secrets.
ISSUER = os.environ.get("AXIOM_ISSUER", "https://auth.axiomcore.dev/00000000-0000-0000-0000-000000000000/00000000-0000-0000-0000-000000000000")
AUDIENCE = os.environ.get("AXIOM_AUDIENCE", ISSUER + "/api")

app = FastAPI(title="Orders API")
app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://127.0.0.1:5176", "http://localhost:5176"],
    allow_methods=["GET"],
    allow_headers=["authorization", "content-type"],
)
verifier = AxiomVerifier(ISSUER, AUDIENCE)


def principal(authorization: Annotated[str | None, Header()] = None) -> Principal:
    if authorization is None or not authorization.startswith("Bearer "):
        raise HTTPException(401, detail="unauthorized", headers={"WWW-Authenticate": "Bearer"})
    try:
        return verifier.verify(authorization[7:])
    except TokenRefused as refused:
        # The reason is a fixed code; the token itself is never logged or echoed.
        raise HTTPException(401, detail="unauthorized", headers={"WWW-Authenticate": f'Bearer error="invalid_token", error_description="{refused.reason}"'}) from None


class Viewer(BaseModel):
    subject: str
    tenant: str | None
    roles: list[str]
    acr: str


@app.get("/me", response_model=Viewer, operation_id="me")
def me(user: Annotated[Principal, Depends(principal)]) -> Viewer:
    return Viewer(subject=user.subject, tenant=user.tenant, roles=user.roles, acr=user.acr)
