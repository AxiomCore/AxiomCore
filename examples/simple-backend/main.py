from fastapi import FastAPI, Depends, HTTPException, status, WebSocket, WebSocketDisconnect, BackgroundTasks, Query
from fastapi.security import OAuth2PasswordBearer, OAuth2PasswordRequestForm
from pydantic import BaseModel, EmailStr
from typing import List, Optional, Dict
from datetime import datetime, timedelta
import jwt
import hashlib
import uuid
import asyncio

# ==============================
# CONFIG
# ==============================
SECRET_KEY = "supersecretkey"
ALGORITHM = "HS256"
ACCESS_TOKEN_EXPIRE_MINUTES = 60

app = FastAPI(title="Complex FastAPI Backend")

oauth2_scheme = OAuth2PasswordBearer(tokenUrl="login")

# ==============================
# IN-MEMORY DATABASE
# ==============================
db_users: Dict[str, dict] = {}
db_items: Dict[str, dict] = {}

# ==============================
# MODELS
# ==============================
class UserCreate(BaseModel):
    email: EmailStr
    password: str

class User(BaseModel):
    id: str
    email: EmailStr
    role: str

class Token(BaseModel):
    access_token: str
    token_type: str

class ItemCreate(BaseModel):
    title: str
    description: Optional[str] = None

class Item(BaseModel):
    id: str
    title: str
    description: Optional[str]
    owner_id: str

# ==============================
# UTILS
# ==============================
def hash_password(password: str):
    return hashlib.sha256(password.encode()).hexdigest()

def verify_password(password: str, hashed: str):
    return hash_password(password) == hashed

def create_access_token(data: dict):
    to_encode = data.copy()
    expire = datetime.utcnow() + timedelta(minutes=ACCESS_TOKEN_EXPIRE_MINUTES)
    to_encode.update({"exp": expire})
    return jwt.encode(to_encode, SECRET_KEY, algorithm=ALGORITHM)

def decode_token(token: str):
    try:
        return jwt.decode(token, SECRET_KEY, algorithms=[ALGORITHM])
    except:
        raise HTTPException(status_code=401, detail="Invalid token")

# ==============================
# AUTH DEPENDENCIES
# ==============================
def get_current_user(token: str = Depends(oauth2_scheme)):
    payload = decode_token(token)
    user_id = payload.get("sub")
    user = db_users.get(user_id)

    if not user:
        raise HTTPException(status_code=401, detail="User not found")

    return user

def require_admin(user=Depends(get_current_user)):
    if user["role"] != "admin":
        raise HTTPException(status_code=403, detail="Admin only")
    return user

# ==============================
# AUTH ROUTES
# ==============================
@app.post("/register", response_model=User)
def register(user: UserCreate):
    user_id = str(uuid.uuid4())

    db_users[user_id] = {
        "id": user_id,
        "email": user.email,
        "password": hash_password(user.password),
        "role": "user"
    }

    return db_users[user_id]

@app.post("/login", response_model=Token)
def login(form_data: OAuth2PasswordRequestForm = Depends()):
    user = next((u for u in db_users.values() if u["email"] == form_data.username), None)

    if not user or not verify_password(form_data.password, user["password"]):
        raise HTTPException(status_code=400, detail="Invalid credentials")

    token = create_access_token({"sub": user["id"]})
    return {"access_token": token, "token_type": "bearer"}

# ==============================
# ITEM CRUD
# ==============================
@app.post("/items", response_model=Item)
def create_item(item: ItemCreate, user=Depends(get_current_user)):
    item_id = str(uuid.uuid4())

    db_items[item_id] = {
        "id": item_id,
        "title": item.title,
        "description": item.description,
        "owner_id": user["id"]
    }

    return db_items[item_id]

@app.get("/items", response_model=List[Item])
def list_items(
    skip: int = 0,
    limit: int = 10,
    search: Optional[str] = Query(None)
):
    items = list(db_items.values())

    if search:
        items = [i for i in items if search.lower() in i["title"].lower()]

    return items[skip: skip + limit]

@app.get("/items/{item_id}", response_model=Item)
def get_item(item_id: str):
    item = db_items.get(item_id)
    if not item:
        raise HTTPException(404, "Item not found")
    return item

@app.delete("/items/{item_id}")
def delete_item(item_id: str, user=Depends(get_current_user)):
    item = db_items.get(item_id)

    if not item:
        raise HTTPException(404, "Item not found")

    if item["owner_id"] != user["id"]:
        raise HTTPException(403, "Not allowed")

    del db_items[item_id]
    return {"message": "Deleted"}

# ==============================
# BACKGROUND TASK
# ==============================
def send_email_mock(email: str):
    print(f"Sending email to {email}...")
    asyncio.sleep(2)
    print("Email sent!")

@app.post("/send-email")
def send_email(background_tasks: BackgroundTasks, user=Depends(get_current_user)):
    background_tasks.add_task(send_email_mock, user["email"])
    return {"message": "Email scheduled"}

# ==============================
# ADMIN ROUTES
# ==============================
@app.get("/admin/users", dependencies=[Depends(require_admin)])
def list_users():
    return list(db_users.values())

# ==============================
# WEBSOCKET CHAT
# ==============================
class ConnectionManager:
    def __init__(self):
        self.active_connections: List[WebSocket] = []

    async def connect(self, websocket: WebSocket):
        await websocket.accept()
        self.active_connections.append(websocket)

    def disconnect(self, websocket: WebSocket):
        self.active_connections.remove(websocket)

    async def broadcast(self, message: str):
        for connection in self.active_connections:
            await connection.send_text(message)

manager = ConnectionManager()

@app.websocket("/ws")
async def websocket_endpoint(websocket: WebSocket):
    await manager.connect(websocket)

    try:
        while True:
            data = await websocket.receive_text()
            await manager.broadcast(f"Message: {data}")
    except WebSocketDisconnect:
        manager.disconnect(websocket)

# ==============================
# MIDDLEWARE (Logging)
# ==============================
@app.middleware("http")
async def log_requests(request, call_next):
    print(f"[{datetime.utcnow()}] {request.method} {request.url}")
    response = await call_next(request)
    return response
