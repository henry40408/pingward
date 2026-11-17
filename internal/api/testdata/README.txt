openapi.json: GET /api/openapi.json from the Rust v0.12.0 image.
responses/*.http: request line, optional "body:" line, then the raw response (status line,
headers minus date/content-length/x-request-id, blank line, body). Cases ran in file order
against the state in ../../db/testdata/README.txt, with a Bearer key. Timestamps, ping UUIDs,
ids of created rows and source_ip vary and must be normalised by the comparison.
Cron error text (check-create-badcron.http) is real Rust output for one expression only; the
full cron/chrono-tz fixture is NOT captured (blocked: needs cargo).
docs.html (/api/docs) NOT captured (needs a session cookie and a running server).
