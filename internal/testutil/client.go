package testutil

import (
	"io"
	"net/http"
	"net/http/cookiejar"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

// Client talks to a handler over a real listener, keeping cookies and never
// following redirects, so tests see each 303 themselves. The listener is on
// 127.0.0.1, which is the one rate-limit bucket every request shares.
type Client struct {
	T       *testing.T
	Server  *httptest.Server
	HTTP    *http.Client
	Headers http.Header
}

func NewClient(t *testing.T, handler http.Handler) *Client {
	t.Helper()
	server := httptest.NewServer(handler)
	t.Cleanup(server.Close)
	jar, err := cookiejar.New(nil)
	if err != nil {
		t.Fatal(err)
	}
	return &Client{
		T: t, Server: server, Headers: http.Header{},
		HTTP: &http.Client{
			Jar: jar,
			CheckRedirect: func(*http.Request, []*http.Request) error {
				return http.ErrUseLastResponse
			},
		},
	}
}

// Response is a fully read response.
type Response struct {
	*http.Response
	Body string
}

func (c *Client) Do(method, path string, body io.Reader, headers http.Header) *Response {
	c.T.Helper()
	req, err := http.NewRequest(method, c.Server.URL+path, body)
	if err != nil {
		c.T.Fatal(err)
	}
	for name, values := range c.Headers {
		req.Header[name] = values
	}
	for name, values := range headers {
		req.Header[name] = values
	}
	res, err := c.HTTP.Do(req)
	if err != nil {
		c.T.Fatal(err)
	}
	defer func() { _ = res.Body.Close() }()
	text, err := io.ReadAll(res.Body)
	if err != nil {
		c.T.Fatal(err)
	}
	return &Response{Response: res, Body: string(text)}
}

func (c *Client) Get(path string) *Response { return c.Do(http.MethodGet, path, nil, nil) }

func (c *Client) PostForm(path string, form url.Values) *Response {
	return c.PostRaw(path, form.Encode(), nil)
}

// PostRaw posts an urlencoded body exactly as given, for malformed-body cases.
func (c *Client) PostRaw(path, body string, headers http.Header) *Response {
	merged := http.Header{"Content-Type": {"application/x-www-form-urlencoded"}}
	for name, values := range headers {
		merged[name] = values
	}
	return c.Do(http.MethodPost, path, strings.NewReader(body), merged)
}

// Cookie is the jar's value for name, or "" when absent.
func (c *Client) Cookie(name string) string {
	base, err := url.Parse(c.Server.URL)
	if err != nil {
		c.T.Fatal(err)
	}
	for _, cookie := range c.HTTP.Jar.Cookies(base) {
		if cookie.Name == name {
			return cookie.Value
		}
	}
	return ""
}

func (c *Client) ClearCookies() {
	jar, err := cookiejar.New(nil)
	if err != nil {
		c.T.Fatal(err)
	}
	c.HTTP.Jar = jar
}
