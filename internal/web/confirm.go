package web

import (
	"net/http"
	"strings"

	"github.com/henry40408/pingward/internal/views"
)

// confirmCopy is a confirmation page's wording, deliberately longer than the
// template's terse data-confirm line.
type confirmCopy struct {
	title, message, button string
}

var (
	confirmRevokeSession = confirmCopy{
		title:   "Revoke this session?",
		message: "The browser holding it is signed out at once. If it is the session you are using now, that includes this one.",
		button:  "Revoke session",
	}
	confirmRevokeOtherSessions = confirmCopy{
		title:   "Revoke every other session?",
		message: "Every other signed-in browser is signed out at once. Only this one stays. API keys are unaffected — they are separate credentials, revoked from the API keys card below.",
		button:  "Revoke the others",
	}
	confirmRevokeAPIKey = confirmCopy{
		title:   "Revoke this API key?",
		message: "Anything using this key stops being able to reach the API immediately. The key cannot be recovered — a replacement is a new key with a new token.",
		button:  "Revoke key",
	}
)

// isConfirmed is true when the query carries a confirmed key at all, even an
// empty one. A query flag, not a body field, so authorization still answers
// before any parsing.
func isConfirmed(r *http.Request) bool { return r.URL.Query().Has("confirmed") }

// confirmationPage re-posts action plus the flag. Call it after authorization
// and every refusal guard: a request that can never succeed should say so
// instead of asking for confirmation.
func (d *Deps) confirmationPage(w http.ResponseWriter, r *http.Request, isAdmin bool, copy confirmCopy, action, cancel string) {
	separator := "?"
	if strings.Contains(action, "?") {
		separator = "&"
	}
	writeHTML(w, http.StatusOK, views.Confirm(views.ConfirmView{
		IsAdmin: isAdmin, CSRF: d.csrfToken(r),
		Title: copy.title, Message: copy.message, Button: copy.button,
		Action: action + separator + "confirmed=1", Cancel: cancel,
	}))
}

// wantsFragment is true only for X-Requested-With: fetch exactly; anything else
// gets the redirect.
func wantsFragment(r *http.Request) bool { return r.Header.Get("X-Requested-With") == "fetch" }
