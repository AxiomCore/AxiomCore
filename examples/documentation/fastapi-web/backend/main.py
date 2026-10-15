from typing import Annotated

from fastapi import FastAPI, Path
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel

app = FastAPI(title="Greeting API")
app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://127.0.0.1:5175", "http://localhost:5175"],
    allow_methods=["GET"],
    allow_headers=["content-type", "traceparent", "x-axiom-cache-mode"],
)


class Greeting(BaseModel):
    message: str


@app.get("/greet/{name}", response_model=Greeting, operation_id="greet")
def greet(name: Annotated[str, Path(min_length=1, max_length=40)]) -> Greeting:
    return Greeting(message=f"Hello, {name}!")
