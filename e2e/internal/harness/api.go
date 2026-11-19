package harness

import (
	"fmt"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"
)

// PingKind is one of the /ping/* endpoints.
type PingKind struct{ suffix string }

// The ping endpoints.
var (
	PingSuccess = PingKind{""}
	PingFail    = PingKind{"/fail"}
	PingStart   = PingKind{"/start"}
	PingLog     = PingKind{"/log"}
)

// PingExit is the /{code} endpoint: 0 succeeds, anything else fails.
func PingExit(code int) PingKind { return PingKind{"/" + strconv.Itoa(code)} }

// ParsePingKind reads a feature-file name (success, fail, exit 3, ...).
func ParsePingKind(name string) (PingKind, error) {
	switch name {
	case "success":
		return PingSuccess, nil
	case "fail":
		return PingFail, nil
	case "start":
		return PingStart, nil
	case "log":
		return PingLog, nil
	}
	code, found := strings.CutPrefix(name, "exit ")
	if !found {
		return PingKind{}, fmt.Errorf("no ping endpoint is called `%s`", name)
	}
	number, err := strconv.Atoi(strings.TrimSpace(code))
	if err != nil {
		return PingKind{}, err
	}
	return PingExit(number), nil
}

// API is the HTTP client the steps drive the server with directly.
type API struct {
	baseURL    string
	setupToken string
	client     *http.Client
}

var csrfField = regexp.MustCompile(`name="_csrf" value="([^"]*)"`)

// NewAPI builds a client for the server. Redirects are asserted, never
// followed.
func NewAPI(server *Server) *API {
	return &API{
		baseURL:    server.BaseURL,
		setupToken: server.SetupToken,
		client: &http.Client{CheckRedirect: func(*http.Request, []*http.Request) error {
			return http.ErrUseLastResponse
		}},
	}
}

// BootstrapAdmin creates the first admin through POST /setup, sending the
// cookie and _csrf from a prior GET as a browser would. Only a success answers
// 303 to /; a validation re-render or a finished setup is an error.
func (a *API) BootstrapAdmin(username, password string) error {
	page, err := a.client.Get(a.baseURL + "/setup")
	if err != nil {
		return fmt.Errorf("GET /setup: %w", err)
	}
	defer func() { _ = page.Body.Close() }()
	html, err := io.ReadAll(page.Body)
	if err != nil {
		return err
	}
	cookies := page.Cookies()
	match := csrfField.FindSubmatch(html)
	if len(cookies) == 0 || match == nil {
		return fmt.Errorf("could not read a session cookie and _csrf token from GET /setup")
	}
	form := url.Values{
		"_csrf":       {string(match[1])},
		"username":    {username},
		"password":    {password},
		"setup_token": {a.setupToken},
	}
	request, err := http.NewRequest(http.MethodPost, a.baseURL+"/setup", strings.NewReader(form.Encode()))
	if err != nil {
		return err
	}
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	request.AddCookie(cookies[0])
	response, err := a.client.Do(request)
	if err != nil {
		return fmt.Errorf("POST /setup: %w", err)
	}
	defer func() { _ = response.Body.Close() }()
	if location := response.Header.Get("Location"); response.StatusCode != http.StatusSeeOther || location != "/" {
		return fmt.Errorf("bootstrapping the admin `%s` answered %d to %q", username, response.StatusCode, location)
	}
	return nil
}

// Ping sends a ping that must be accepted; PingStatus reports refusals.
func (a *API) Ping(pingURL string, kind PingKind) error {
	status, err := a.PingStatus(pingURL, kind)
	if err != nil {
		return err
	}
	if status < 200 || status >= 300 {
		return fmt.Errorf("a ping to %s%s answered %d", pingURL, kind.suffix, status)
	}
	return nil
}

// PingStatus sends a ping and returns its status, even a refusal.
func (a *API) PingStatus(pingURL string, kind PingKind) (int, error) {
	return a.status(http.MethodGet, pingURL+kind.suffix, "")
}

// PingWithBody POSTs a ping with a body; it must be accepted.
func (a *API) PingWithBody(pingURL string, kind PingKind, body string) error {
	status, err := a.status(http.MethodPost, pingURL+kind.suffix, body)
	if err != nil {
		return err
	}
	if status < 200 || status >= 300 {
		return fmt.Errorf("a ping with a body to %s%s answered %d", pingURL, kind.suffix, status)
	}
	return nil
}

// StatusOf GETs a path and reports its status.
func (a *API) StatusOf(path string) (int, error) {
	return a.status(http.MethodGet, a.baseURL+path, "")
}

// PostFormStatus POSTs a form without cookie or CSRF token (for the CSRF
// scenarios) and reports its status.
func (a *API) PostFormStatus(path string, form url.Values) (int, error) {
	return a.status(http.MethodPost, a.baseURL+path, form.Encode())
}

func (a *API) status(method, target, body string) (int, error) {
	request, err := http.NewRequest(method, target, strings.NewReader(body))
	if err != nil {
		return 0, err
	}
	if method == http.MethodPost && strings.Contains(body, "=") {
		request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	}
	response, err := a.client.Do(request)
	if err != nil {
		return 0, fmt.Errorf("%s %s: %w", method, target, err)
	}
	defer func() { _ = response.Body.Close() }()
	_, _ = io.Copy(io.Discard, response.Body)
	return response.StatusCode, nil
}

// PostFormAs POSTs a form with the given Cookie header, following no
// redirects. It is not a fetch: redirect:'manual' reads a 303 as 0, and
// self-guard scenarios must tell it from a CSRF 403.
func (a *API) PostFormAs(cookieHeader, path string, form url.Values) (int, error) {
	request, err := http.NewRequest(http.MethodPost, a.baseURL+path, strings.NewReader(form.Encode()))
	if err != nil {
		return 0, err
	}
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	request.Header.Set("Cookie", cookieHeader)
	response, err := a.client.Do(request)
	if err != nil {
		return 0, fmt.Errorf("posting %s as the signed-in user: %w", path, err)
	}
	defer func() { _ = response.Body.Close() }()
	return response.StatusCode, nil
}
