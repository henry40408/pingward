-- The /admin audit table filters by actor and action, and builds its two
-- filter selects from `SELECT DISTINCT` over the same columns. Nothing prunes
-- audit_log, so it only grows; without these both the filter and the select
-- population degrade into full scans on every /admin render.
CREATE INDEX idx_audit_actor ON audit_log(actor_username);
CREATE INDEX idx_audit_action ON audit_log(action);
