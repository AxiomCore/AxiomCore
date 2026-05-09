import time
from typing import List

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel

app = FastAPI()


app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


class Post(BaseModel):
    id: int
    author: str
    content: str
    timestamp: str


@app.get("/posts", response_model=List[Post])
def get_posts():
    # We append the current server time so you can visually see when the cache updates!
    current_time = time.strftime("%X")
    return [
        Post(
            id=1,
            author="Alice",
            content="Just deployed my first Axiom app!",
            timestamp=current_time,
        ),
        Post(
            id=2,
            author="Bob",
            content="The stale-while-revalidate cache is blazing fast.",
            timestamp=current_time,
        ),
        Post(
            id=3,
            author="Charlie",
            content="Network is flakey today, good thing we have Retries!",
            timestamp=current_time,
        ),
    ]
