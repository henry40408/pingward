package web

import "net/http"

func notFound(w http.ResponseWriter) { http.Error(w, "not found", http.StatusNotFound) }

func internalError(w http.ResponseWriter) {
	http.Error(w, "internal error", http.StatusInternalServerError)
}

func adminOnly(w http.ResponseWriter) { http.Error(w, "admin only", http.StatusForbidden) }

// redirect is always 303, like axum's Redirect::to.
func redirect(w http.ResponseWriter, r *http.Request, location string) {
	http.Redirect(w, r, location, http.StatusSeeOther)
}
