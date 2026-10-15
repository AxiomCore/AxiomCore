package main

import (
    "encoding/json"
    "log"
    "net/http"
    "time"
    "unicode/utf8"

    "github.com/go-chi/chi/v5"
)

type Greeting struct { Message string `json:"message"` }

func greet(w http.ResponseWriter, r *http.Request) {
    name := chi.URLParam(r, "name")
    if utf8.RuneCountInString(name) > 40 {
        http.Error(w, "Name must contain at most 40 characters", http.StatusUnprocessableEntity)
        return
    }
    w.Header().Set("Content-Type", "application/json")
    _ = json.NewEncoder(w).Encode(Greeting{Message: "Hello, " + name + "!"})
}

func main() {
    router := chi.NewRouter()
    router.Use(func(next http.Handler) http.Handler {
        return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
            origin := r.Header.Get("Origin")
            if origin == "http://127.0.0.1:5176" || origin == "http://localhost:5176" {
                w.Header().Set("Access-Control-Allow-Origin", origin)
                w.Header().Set("Vary", "Origin")
                w.Header().Set("Access-Control-Allow-Methods", "GET, OPTIONS")
                w.Header().Set("Access-Control-Allow-Headers", "content-type, traceparent, x-axiom-cache-mode")
            }
            if r.Method == http.MethodOptions { w.WriteHeader(http.StatusNoContent); return }
            next.ServeHTTP(w, r)
        })
    })
    router.Get("/greet/{name}", greet)
    server := &http.Server{Addr: "127.0.0.1:8084", Handler: router, ReadHeaderTimeout: 5 * time.Second}
    log.Fatal(server.ListenAndServe())
}
