// Package assets embeds the hand-written front-end files and serves them.
package assets

import (
	"embed"
	"fmt"
	"hash/fnv"
	"net/http"
	"strings"
)

//go:embed static
var files embed.FS

// fontPlaceholder sits inside a quoted url("…") in app.css so the source file
// stays valid CSS; it is replaced by the font version at startup.
const fontPlaceholder = "{{FONT_V}}"

// immutableCache is safe because every asset URL carries a content hash (?v=).
const immutableCache = "public, max-age=31536000, immutable"

type asset struct {
	name        string
	contentType string
	body        []byte
}

// One table per kind, so the version hash and the handler agree on what is served.
var (
	fonts = loadAssets("font/woff2", "fonts/",
		"inter-400.woff2", "inter-500.woff2", "inter-600.woff2", "inter-700.woff2",
		"ibm-plex-mono-400.woff2", "ibm-plex-mono-500.woff2", "ibm-plex-mono-600.woff2")
	// Files, not inline <script>, so the CSP can stay script-src 'self'.
	scripts = loadAssets("text/javascript; charset=utf-8", "", "app.js", "theme-init.js")
	// apple-touch-icon.png is rendered from favicon.svg (see e2e's icons binary).
	icons = []asset{
		{"favicon.svg", "image/svg+xml", read("favicon.svg")},
		{"apple-touch-icon.png", "image/png", read("apple-touch-icon.png")},
	}

	fontVersion = hashAssets(fonts)
	appCSS      = strings.ReplaceAll(string(read("app.css")), fontPlaceholder, fontVersion)
	// The stylesheet hash also changes with the fonts. These hashes are not
	// cryptographic; instability across toolchains costs one refetch.
	cssVersion  = hashBytes([]byte(appCSS))
	iconVersion = hashAssets(icons)
	// One version for both scripts, which ship together.
	jsVersion = hashAssets(scripts)
)

// CSSVersion is the ?v= value for /assets/app.css.
func CSSVersion() string { return cssVersion }

// JSVersion is the ?v= value for the scripts.
func JSVersion() string { return jsVersion }

// IconVersion is the ?v= value for the favicon and apple-touch-icon.
func IconVersion() string { return iconVersion }

// FontVersion is the ?v= value baked into the stylesheet's font URLs.
func FontVersion() string { return fontVersion }

func read(name string) []byte {
	body, err := files.ReadFile("static/" + name)
	if err != nil {
		panic("assets: embedded file missing: " + err.Error())
	}
	return body
}

func loadAssets(contentType, dir string, names ...string) []asset {
	loaded := make([]asset, len(names))
	for i, name := range names {
		loaded[i] = asset{name, contentType, read(dir + name)}
	}
	return loaded
}

func hashAssets(assets []asset) string {
	hasher := fnv.New64a()
	for _, a := range assets {
		hasher.Write([]byte(a.name))
		hasher.Write(a.body)
	}
	return fmt.Sprintf("%x", hasher.Sum64())
}

func hashBytes(body []byte) string {
	hasher := fnv.New64a()
	hasher.Write(body)
	return fmt.Sprintf("%x", hasher.Sum64())
}

// Handler serves /assets/*, /favicon.svg and /apple-touch-icon.png. At the
// root because browsers and iOS probe those paths without a <link>.
func Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /assets/app.css", serveBody("text/css; charset=utf-8", []byte(appCSS)))
	mux.HandleFunc("GET /assets/{file}", serveNamed(scripts))
	mux.HandleFunc("GET /assets/fonts/{file}", serveNamed(fonts))
	mux.HandleFunc("GET /favicon.svg", serveByPath(icons))
	mux.HandleFunc("GET /apple-touch-icon.png", serveByPath(icons))
	return mux
}

func serveBody(contentType string, body []byte) http.HandlerFunc {
	return func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", contentType)
		w.Header().Set("Cache-Control", immutableCache)
		_, _ = w.Write(body)
	}
}

func serveNamed(assets []asset) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		serveFound(w, r, assets, r.PathValue("file"))
	}
}

func serveByPath(assets []asset) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		serveFound(w, r, assets, strings.TrimPrefix(r.URL.Path, "/"))
	}
}

func serveFound(w http.ResponseWriter, r *http.Request, assets []asset, name string) {
	for _, a := range assets {
		if a.name == name {
			serveBody(a.contentType, a.body)(w, r)
			return
		}
	}
	w.WriteHeader(http.StatusNotFound)
}
