vectors.json: session cookie + derived CSRF token pairs from a live Rust v0.12.0 server with the
given PINGWARD_SECRET (raw UTF-8 bytes). Flash cookie and setup-token vectors were not captured;
the Go tests derive them from secret.rs semantics instead (list as a deviation).
