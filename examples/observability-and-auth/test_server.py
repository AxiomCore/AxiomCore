import datetime

import jwt  # pip install pyjwt
from fastapi import FastAPI, Header, HTTPException, Query

app = FastAPI()

SECRET_KEY = "my_super_secret_key"
ALGORITHM = "HS256"


@app.post("/login")
def login():
    # Provide the scope required by the client contract
    payload = {
        "sub": "user_123",
        "scopes": ["items:read", "profile:write"],
        "exp": datetime.datetime.utcnow() + datetime.timedelta(hours=1),
    }
    token = jwt.encode(payload, SECRET_KEY, algorithm=ALGORITHM)
    return {"token": token}


@app.get("/protected/jwt")
def protected_jwt(authorization: str = Header(None)):
    # Axiom Engine intercepts and verifies the JWT before the network call!
    return {"message": "Successfully accessed a JWT protected route"}


@app.get("/protected/api-key-header")
def protected_api_key_header(x_api_key: str = Header(None)):
    if x_api_key != "secret-key-123":
        raise HTTPException(status_code=403, detail="Invalid API Key")
    return {"message": "You accessed an API Key Header protected route"}


@app.get("/protected/api-key-query")
def protected_api_key_query(api_key: str = Query(None)):
    if api_key != "secret-key-123":
        raise HTTPException(status_code=403, detail="Invalid API Key")
    return {"message": "You accessed an API Key Query protected route"}


@app.get("/protected/multi")
def protected_multi(authorization: str = Header(None), x_api_key: str = Header(None)):
    return {"message": "You passed the AND/OR multi-auth condition!"}
