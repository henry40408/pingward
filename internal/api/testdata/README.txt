openapi.json: GET /api/openapi.json from the Rust v0.12.0 image.
docs.html: GET /api/docs with a login session, from the same v0.12.0 image (captured 2026-11-18).
openapi.json was re-captured the same day; the embedded copy in ../openapi.json is identical.
The four stateless error responses the server tests compare against are inline in
internal/server/api_v1_test.go (rustCaptures).
