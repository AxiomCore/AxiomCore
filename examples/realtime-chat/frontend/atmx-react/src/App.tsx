import { useState, useEffect, useRef } from "react";
import { sdk } from "./generated/sdk";
import type { realtimeChat } from "./generated/models";

export default function App() {
  const [messages, setMessages] = useState<realtimeChat.ChatMessage[]>([]);
  const [sender, setSender] = useState("ReactUser");
  const [content, setContent] = useState("");
  const chatWindowRef = useRef<HTMLDivElement>(null);

  const { data } = sdk.realtimeChat.useHandleConnections();

  useEffect(() => {
    if (data) {
      setTimeout(() => {
        setMessages((prev) => [
          ...prev,
          data as unknown as realtimeChat.ChatMessage,
        ]);
        if (chatWindowRef.current) {
          chatWindowRef.current.scrollTop = chatWindowRef.current.scrollHeight;
        }
      }, 0);
    }
  }, [data]);

  const handleSend = () => {
    if (!content.trim()) return;

    sdk.realtimeChat.axiom.send("handleConnections", {
      sender,
      content,
    });

    setContent("");
  };

  return (
    <div className="bg-slate-100 min-h-screen text-slate-800 p-8 font-sans flex justify-center">
      <div className="w-full max-w-2xl">
        {/* Navigation Bar */}
        <nav className="flex justify-between items-center bg-white p-4 rounded-xl shadow-sm mb-6 border border-slate-200">
          <div className="font-bold text-xl text-blue-600">
            Axiom React Chat
          </div>
          <div
            className={`text-xs font-bold ${status === "loading" ? "text-amber-500" : "text-green-600"}`}
          >
            {status === "loading" ? "Connecting..." : "✅ Engine Online"}
          </div>
        </nav>

        {/* Chat Container */}
        <div className="bg-white rounded-xl shadow-sm border border-slate-200 overflow-hidden flex flex-col h-[600px]">
          <div className="bg-slate-800 text-white p-4 font-bold text-sm">
            Live Global Chat Room
          </div>

          {/* Messages Window */}
          <div
            ref={chatWindowRef}
            className="flex-1 p-4 overflow-y-auto space-y-4 bg-white"
          >
            {status === "loading" && (
              <div className="text-center text-slate-400 text-xs py-4 animate-pulse">
                Connecting to WebSocket via Rust WASM...
              </div>
            )}

            <ul className="space-y-3">
              {messages.map((msg, idx) => {
                const isMe = msg.sender === sender;
                const isSystem = msg.sender === "System";

                return (
                  <li
                    key={idx}
                    className={`flex ${isSystem ? "justify-center" : isMe ? "justify-end" : "justify-start"}`}
                  >
                    <div
                      className={`max-w-[80%] p-3 rounded-xl shadow-sm ${
                        isSystem
                          ? "bg-slate-100 text-slate-500 text-center italic"
                          : isMe
                            ? "bg-blue-600 text-white"
                            : "bg-slate-100 text-slate-800 border border-slate-200"
                      }`}
                    >
                      {!isSystem && (
                        <div
                          className={`text-[10px] mb-1 font-bold uppercase tracking-wider ${isMe ? "text-blue-100" : "text-slate-400"}`}
                        >
                          {msg.sender}
                        </div>
                      )}
                      <div className="text-sm leading-relaxed">
                        {msg.content}
                      </div>
                    </div>
                  </li>
                );
              })}
            </ul>
          </div>

          {/* Input Area */}
          <div className="p-4 bg-slate-50 border-t border-slate-200 flex gap-2">
            <input
              type="text"
              value={sender}
              onChange={(e) => setSender(e.target.value)}
              className="w-1/4 p-3 rounded-lg border border-slate-300 outline-none bg-white text-slate-900 focus:border-blue-500 transition-colors"
              placeholder="Name"
            />
            <input
              type="text"
              value={content}
              onChange={(e) => setContent(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleSend()}
              className="w-full p-3 rounded-lg border border-slate-300 outline-none bg-white text-slate-900 focus:border-blue-500 transition-colors"
              placeholder="Type a message..."
            />
            <button
              onClick={handleSend}
              className="bg-blue-600 text-white font-bold px-6 py-3 rounded-lg hover:bg-blue-700 transition active:scale-95 shadow-md shadow-blue-200"
            >
              Send
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
