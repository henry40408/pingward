package config

import "github.com/henry40408/pingward/internal/secret"

// EphemeralSecretWarning explains a generated secret; empty when the secret
// came from the environment. Otherwise the only symptom of an unset secret is
// an unexplained logout on restart.
func EphemeralSecretWarning(source secret.Source) string {
	var cause string
	switch source {
	case secret.SourceGenerated:
		cause = "PINGWARD_SECRET is not set"
	case secret.SourceRejected:
		cause = "PINGWARD_SECRET is shorter than the 16-byte minimum and was ignored"
	default:
		return ""
	}
	return cause + "; using a secret generated for this process only. Every signed-in " +
		"browser session will end on restart. Set PINGWARD_SECRET (e.g. `openssl rand -hex 32`) " +
		"to keep sessions across restarts. API keys are unaffected."
}

// OrphanLogoutURLWarning is a warning rather than a refusal, so gateway
// config can be staged in two steps.
func OrphanLogoutURLWarning(c Config) string {
	if c.ForwardAuthLogout == nil || c.ForwardAuthHeader != nil {
		return ""
	}
	return "PINGWARD_FORWARD_AUTH_LOGOUT_URL is set but PINGWARD_FORWARD_AUTH_HEADER is not; " +
		"logging out will still redirect there, but no request is authenticated by a " +
		"gateway header."
}
