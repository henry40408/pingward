package api

import (
	"bytes"
	"encoding/json"
	"net/http"
)

type errorBody struct {
	Error errorDetail `json:"error"`
}

type errorDetail struct {
	// Code is stable and machine-readable, for clients to branch on.
	Code    string `json:"code"`
	Message string `json:"message"`
}

// writeJSON matches serde_json's output: no HTML escaping and no trailing newline.
func writeJSON(w http.ResponseWriter, status int, value any) {
	var out bytes.Buffer
	encoder := json.NewEncoder(&out)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(value); err != nil {
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_, _ = w.Write(bytes.TrimSuffix(out.Bytes(), []byte("\n")))
}

func writeError(w http.ResponseWriter, status int, code, message string) {
	writeJSON(w, status, errorBody{errorDetail{Code: code, Message: message}})
}

// notFound answers both "does not exist" and "not yours", so existence never leaks.
func notFound(w http.ResponseWriter) {
	writeError(w, http.StatusNotFound, "not_found", "resource not found")
}

func unauthorized(w http.ResponseWriter) {
	writeError(w, http.StatusUnauthorized, "unauthorized", "missing or invalid API key")
}

func badRequest(w http.ResponseWriter, message string) {
	writeError(w, http.StatusBadRequest, "bad_request", message)
}

func (a *API) internalError(w http.ResponseWriter, err error) {
	a.d.Logger.Error("api db error", "error", err)
	writeError(w, http.StatusInternalServerError, "internal", "internal error")
}

// plainText answers like axum's built-in rejections: text/plain with no trailing newline.
func plainText(w http.ResponseWriter, status int, message string) {
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	w.WriteHeader(status)
	_, _ = w.Write([]byte(message))
}
