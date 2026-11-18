package web

import (
	"net/http"
	"strconv"
)

func writeHTML(w http.ResponseWriter, status int, page string) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.WriteHeader(status)
	_, _ = w.Write([]byte(page))
}

func setRetryAfter(w http.ResponseWriter, secs int64) {
	w.Header().Set("Retry-After", strconv.FormatInt(secs, 10))
}
