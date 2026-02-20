from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from typing import List, Optional
from enum import Enum

app = FastAPI()

class UserRole(str, Enum):
    admin = "admin"
    user = "user"

class User(BaseModel):
    id: int
    name: str
    role: Optional[UserRole] = None
    email: str

class Message(BaseModel):
    message: str

@app.get("/users/{user_id}", response_model=User)
def get_user(user_id: int):
    """Gets a single user by ID."""
    return {"id": user_id, "name": "John Doe", "role": UserRole.admin, "email": 'test@gmail.com'}

@app.get("/users", response_model=List[User])
def list_users(limit: int = 10):
    """Lists all users."""
    return [{"id": 1, "name": "John Doe", "role": UserRole.admin, "email": 'test@gmail.com'}, {"id": 2, "name": "Jane Doe", "role": UserRole.user, "email": 'test@gmail.com'}]

@app.get("/_internal", include_in_schema=False)
def internal_route():
    """This route should not be included in the schema."""
    return {"status": "ok"}