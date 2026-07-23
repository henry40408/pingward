-- Session cookies are now `<id>.<hmac>` and the CSRF token is derived from the
-- session id (see src/secret.rs), so the per-session column is redundant.
-- Existing cookies carry no signature and can never verify, so their rows are
-- dead weight: clear them in the same step rather than leaving unusable
-- sessions listed on /account. Everyone signs in again once, on upgrade.
DELETE FROM sessions;
ALTER TABLE sessions DROP COLUMN csrf_token;
