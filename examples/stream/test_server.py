import asyncio
import json
from typing import AsyncGenerator

from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse
from pydantic import BaseModel

app = FastAPI()

# Enable CORS for Flutter Web testing
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_methods=["*"],
    allow_headers=["*"],
)


class ChatMessage(BaseModel):
    user: str
    text: str


# 1. HTTP Chunked Stream (NDJSON)
async def number_generator() -> AsyncGenerator[str, None]:
    for i in range(1, 6):
        msg = ChatMessage(user="system", text=f"Message chunk #{i}")
        yield msg.model_dump_json() + "\n"  # ✅ convert to JSON string
        await asyncio.sleep(1)


@app.get("/stream/chunks")
async def stream_chunks():
    return StreamingResponse(number_generator(), media_type="application/x-ndjson")


# 2. Server-Sent Events (SSE)
async def sse_generator():
    for i in range(1, 6):
        yield f"data: {json.dumps({'user': 'sse-bot', 'text': f'Event {i}'})}\n\n"
        await asyncio.sleep(0.8)


@app.get("/stream/sse")
async def stream_sse():
    return StreamingResponse(sse_generator(), media_type="text/event-stream")


# 3. Bidirectional WebSocket
@app.websocket("/ws/chat")
async def websocket_endpoint(websocket: WebSocket):
    await websocket.accept()
    try:
        while True:
            # Receive text from Flutter
            data = await websocket.receive_text()
            # Echo back a response
            await websocket.send_json({"user": "EchoBot", "text": f"Received: {data}"})
    except WebSocketDisconnect:
        print("Client disconnected")


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=8000)
