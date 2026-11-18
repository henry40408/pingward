package api

import (
	_ "embed"
	"net/http"
)

// openAPIJSON is the document the Rust build generated with utoipa, captured
// from v0.12.0. Routes and paths are held together by TestOpenAPIMatchesRoutes.
//
//go:embed openapi.json
var openAPIJSON []byte

// referencePageHead and referencePageTail wrap the document in the Scalar page
// utoipa-scalar rendered; Scalar itself loads from a CDN.
const (
	referencePageHead = `<!doctype html>
<html>
<head>
    <title>Scalar</title>
    <meta charset="utf-8"/>
    <meta
            name="viewport"
            content="width=device-width, initial-scale=1"/>
</head>
<body>

<script
        id="api-reference"
        type="application/json">
    `
	referencePageTail = `
</script>
<script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
</body>
</html>
`
)

// The reference is not public: both pages need a signed-in web session (the
// bearer key is for /api/v1 only) and redirect to /login without one.

func (a *API) openAPIDocument(w http.ResponseWriter, r *http.Request) {
	if _, ok := a.d.RequireUser(w, r); !ok {
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_, _ = w.Write(openAPIJSON)
}

func (a *API) referencePage(w http.ResponseWriter, r *http.Request) {
	if _, ok := a.d.RequireUser(w, r); !ok {
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	_, _ = w.Write([]byte(referencePageHead))
	_, _ = w.Write(openAPIJSON)
	_, _ = w.Write([]byte(referencePageTail))
}
