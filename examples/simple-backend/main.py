import asyncio
from fastapi import FastAPI
from pydantic import BaseModel
from fastapi.middleware.cors import CORSMiddleware
from typing import List, Optional
from enum import Enum

app = FastAPI()

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

class UserRole(str, Enum):
    admin = "admin"
    user = "user"

class User(BaseModel):
    id: int
    name: str
    role: Optional[UserRole] = None
    email: str

@app.get("/users/{user_id}", response_model=User)
async def get_user(user_id: int):
    """Gets a single user by ID."""
    await asyncio.sleep(1.5) # ⏳ ARTIFICIAL DELAY TO PROVE RUST CACHING!
    return {"id": user_id, "name": "John Doe", "role": UserRole.admin, "email": 'john@axiom.dev'}

@app.post("/users", response_model=User)
async def create_user(user: User):
    """Creates a user (Target for our ax-mutate test)"""
    await asyncio.sleep(1) # Simulate DB write
    return user