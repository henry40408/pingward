*.golden.html: raw responses of the Rust v0.12.0 server (ghcr.io/henry40408/pingward:latest,
revision ca5682f) over HTTP, rendered from the same state as ../../db/testdata/rust-created.sqlite3
(admin, bob, carol; see that README). Admin pages come from an elevated admin session.
Fragment endpoints (pings, notifications, audit) were requested with `X-Requested-With: fetch`.
Confirmation pages are the POST response without ?confirmed, with a valid CSRF token.
Values that vary between runs must be normalised by the Go comparison: _csrf value,
asset ?v= hashes, timestamps and relative times ("3s ago"), ping UUIDs, version in the footer.
Not captured (needs other server runs): setup with SMTP disabled vs enabled variants of
channel_form (the capture server had SMTP configured), forward-auth account page, passwordless
account, dashboard filters beyond down/search, project/check pages for every status.
