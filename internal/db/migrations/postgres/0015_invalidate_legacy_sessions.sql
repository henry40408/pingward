-- The 72-hour idle window cannot be applied retroactively to sessions written
-- before it existed: their expires_at was never maintained as "last activity
-- + 72h", so a pre-upgrade row would stay resolvable under its old fixed
-- 30-day expiry until something happened to touch it once (see
-- auth::refreshed_expiry's downward clamp). Deleting those rows outright
-- removes the whole class of problem rather than narrowing it, and this
-- release already invalidates cookies on HTTPS deployments via the __Host-
-- cookie rename, so this costs one extra logout that most operators are
-- already taking regardless. Everyone signs in again once, on upgrade.
DELETE FROM sessions;
