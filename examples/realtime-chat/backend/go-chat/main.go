package main

import (
	"log"
	"net/http"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

type ChatMessage struct {
	Sender    string `json:"sender"`
	Content   string `json:"content"`
	Timestamp string `json:"timestamp"`
}

var (
	clients   = make(map[*websocket.Conn]bool)
	clientsMu sync.Mutex
	upgrader  = websocket.Upgrader{
		CheckOrigin: func(r *http.Request) bool { return true }, // Allow all origins for demo
	}
)

func handleConnections(w http.ResponseWriter, r *http.Request) {
	ws, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		log.Fatal(err)
	}
	defer ws.Close()

	clientsMu.Lock()
	clients[ws] = true
	clientsMu.Unlock()

	log.Println("✅ Client connected")

	// ✨ FIX 1: Send an immediate welcome message!
	// This will trigger ax-on:data on the frontend and hide the loading spinner.
	welcome := ChatMessage{
		Sender:    "System",
		Content:   "Welcome to the Live Global Chat Room!",
		Timestamp: time.Now().Format(time.RFC3339),
	}
	ws.WriteJSON(welcome)

	for {
		var msg ChatMessage
		err := ws.ReadJSON(&msg)
		if err != nil {
			log.Printf("Client disconnected: %v", err)
			clientsMu.Lock()
			delete(clients, ws)
			clientsMu.Unlock()
			break
		}

		msg.Timestamp = time.Now().Format(time.RFC3339)
		log.Printf("📥 Received: %s: %s", msg.Sender, msg.Content)

		// Broadcast
		clientsMu.Lock()
		for client := range clients {
			err := client.WriteJSON(msg)
			if err != nil {
				log.Printf("Error writing to client: %v", err)
				client.Close()
				delete(clients, client)
			}
		}
		clientsMu.Unlock()
	}
}

func main() {
	http.HandleFunc("/ws", handleConnections)
	log.Println("🚀 Chat Server started on :8080")
	err := http.ListenAndServe(":8080", nil)
	if err != nil {
		log.Fatal("ListenAndServe: ", err)
	}
}
