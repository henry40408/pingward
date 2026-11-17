package views

import (
	"github.com/henry40408/pingward/internal/assets"
	"github.com/henry40408/pingward/internal/version"
)

// Chrome is what the page chrome needs from the session.
type Chrome struct {
	ShowNav bool
	IsAdmin bool
	CSRF    string
}

// Page is the shared document: head, optional nav bar, body inside <main>, footer.
// No data-theme on purpose: theme-init.js sets it, and without script its
// absence lets app.css's prefers-color-scheme block answer.
func Page(chrome Chrome, body node) string {
	iconVersion := assets.IconVersion()
	jsVersion := assets.JSVersion()
	headNode := el("head", nil,
		void("meta", a("charset", "utf-8")),
		void("meta", a("name", "viewport"), a("content", "width=device-width, initial-scale=1")),
		el("title", nil, text("pingward")),
		void("link", a("rel", "stylesheet"), a("href", "/assets/app.css?v="+assets.CSSVersion())),
		void("link", a("rel", "icon"), a("type", "image/svg+xml"), a("href", "/favicon.svg?v="+iconVersion)),
		void("link", a("rel", "apple-touch-icon"), a("href", "/apple-touch-icon.png?v="+iconVersion)),
		// Render-blocking on purpose (theme before first paint). Files, not inline
		// blocks, so the CSP can be script-src 'self'.
		el("script", []attr{a("src", "/assets/theme-init.js?v="+jsVersion)}),
		el("script", []attr{a("src", "/assets/app.js?v="+jsVersion), flag("defer")}),
	)
	children := []node{}
	if chrome.ShowNav {
		children = append(children, nav(chrome))
	}
	children = append(children,
		el("main", []attr{a("class", "wrap")}, body),
		// Outside ShowNav so login and setup show the version too.
		el("footer", []attr{a("class", "foot")},
			el("span", []attr{a("data-testid", "app-version")}, text("pingward "+version.Version()))),
	)
	return string(group(
		raw("<!DOCTYPE html>"),
		el("html", []attr{a("lang", "en")}, headNode, el("body", nil, children...)),
	))
}

func nav(chrome Chrome) node {
	links := []node{
		el("a", []attr{a("href", "/")}, text("Dashboard")),
		el("a", []attr{a("href", "/account"), a("data-testid", "nav-account")}, text("Account")),
	}
	if chrome.IsAdmin {
		links = append(links, el("a", []attr{a("href", "/admin"), a("data-testid", "nav-admin")}, text("Admin")))
	}
	return el("header", []attr{a("class", "bar")}, el("div", []attr{a("class", "inner")},
		el("a", []attr{a("class", "brand"), a("href", "/")},
			el("span", []attr{a("class", "glyph")}, text("▚")), text("pingward")),
		el("nav", []attr{a("class", "links")}, links...),
		el("div", []attr{a("class", "right")},
			el("button", []attr{a("class", "iconbtn"), a("id", "pw-theme-toggle"), a("title", "Toggle theme"), a("aria-label", "Toggle theme")}, text("◐")),
			el("form", []attr{a("class", "inline"), a("method", "post"), a("action", "/logout")},
				CSRFField(chrome.CSRF),
				el("button", []attr{a("class", "ghost"), a("type", "submit"), a("data-testid", "logout-button")}, text("Log out"))),
		),
	))
}

// CSRFField is the hidden _csrf input every POST form carries.
func CSRFField(csrf string) node {
	return void("input", a("type", "hidden"), a("name", "_csrf"), a("value", csrf))
}
