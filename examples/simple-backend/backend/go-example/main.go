package main

import (
	"encoding/json"
	"log"
	"net/http"
	"strconv"

	"github.com/go-chi/chi/v5"
)

// Item represents a simple resource
type Item struct {
	ID   int    `json:"id"`
	Name string `json:"name"`
}

// In-memory store
var items = make(map[int]Item)
var idCounter = 1

func main() {
	r := chi.NewRouter()

	r.Route("/items", func(r chi.Router) {
		r.Get("/", getItems)
		r.Post("/", createItem)
		r.Get("/{id}", getItem)
		r.Put("/{id}", updateItem)
		r.Delete("/{id}", deleteItem)
	})

	log.Println("Server running on :8080")
	http.ListenAndServe(":8080", r)
}

// Handlers

func getItems(w http.ResponseWriter, r *http.Request) {
	var list []Item
	for _, item := range items {
		list = append(list, item)
	}
	json.NewEncoder(w).Encode(list)
}

func createItem(w http.ResponseWriter, r *http.Request) {
	var item Item
	if err := json.NewDecoder(r.Body).Decode(&item); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	item.ID = idCounter
	idCounter++
	items[item.ID] = item

	w.WriteHeader(http.StatusCreated)
	json.NewEncoder(w).Encode(item)
}

func getItem(w http.ResponseWriter, r *http.Request) {
	id, _ := strconv.Atoi(chi.URLParam(r, "id"))

	item, exists := items[id]
	if !exists {
		http.NotFound(w, r)
		return
	}

	json.NewEncoder(w).Encode(item)
}

func updateItem(w http.ResponseWriter, r *http.Request) {
	id, _ := strconv.Atoi(chi.URLParam(r, "id"))

	_, exists := items[id]
	if !exists {
		http.NotFound(w, r)
		return
	}

	var updated Item
	if err := json.NewDecoder(r.Body).Decode(&updated); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	updated.ID = id
	items[id] = updated

	json.NewEncoder(w).Encode(updated)
}

func deleteItem(w http.ResponseWriter, r *http.Request) {
	id, _ := strconv.Atoi(chi.URLParam(r, "id"))

	_, exists := items[id]
	if !exists {
		http.NotFound(w, r)
		return
	}

	delete(items, id)
	w.WriteHeader(http.StatusNoContent)
}
