package config

import (
	"fmt"
	"strconv"
	"strings"
)

// TLSMode is how the SMTP connection is secured.
type TLSMode int

const (
	TLSStartTLS TLSMode = iota
	TLSImplicit
	TLSNone
)

// SMTPConfig is the instance-wide mail server. It is enabled only when both
// host and from are set.
type SMTPConfig struct {
	Host     string
	Port     uint16
	Username *string
	Password *string
	From     string
	TLS      TLSMode
}

// String keeps the password out of logs.
func (s SMTPConfig) String() string {
	return fmt.Sprintf("smtp{host=%q port=%d from=%q tls=%d auth=%t}",
		s.Host, s.Port, s.From, s.TLS, s.Username != nil && s.Password != nil)
}

func (s SMTPConfig) GoString() string { return s.String() }

const unencryptedAuthWarning = "SMTP AUTH credentials set with PINGWARD_SMTP_TLS=none: " +
	"credentials will be sent unencrypted"

func loadSMTP(env environment) (*SMTPConfig, string) {
	host, from := env.trimmed("PINGWARD_SMTP_HOST"), env.trimmed("PINGWARD_SMTP_FROM")
	if host == "" || from == "" {
		return nil, ""
	}
	mode := parseTLSMode(env.trimmed("PINGWARD_SMTP_TLS"))
	smtp := &SMTPConfig{
		Host:     host,
		From:     from,
		TLS:      mode,
		Port:     parsePort(env.trimmed("PINGWARD_SMTP_PORT"), defaultPort(mode)),
		Username: env.optionalNonBlank("PINGWARD_SMTP_USERNAME"),
		Password: env.optionalNonBlank("PINGWARD_SMTP_PASSWORD"),
	}
	if mode == TLSNone && smtp.Username != nil && smtp.Password != nil {
		return smtp, unencryptedAuthWarning
	}
	return smtp, ""
}

// parseTLSMode treats anything unknown as STARTTLS.
func parseTLSMode(raw string) TLSMode {
	switch strings.ToLower(raw) {
	case "tls":
		return TLSImplicit
	case "none":
		return TLSNone
	}
	return TLSStartTLS
}

func defaultPort(mode TLSMode) uint16 {
	if mode == TLSImplicit {
		return 465
	}
	return 587
}

func parsePort(raw string, fallback uint16) uint16 {
	port, err := strconv.ParseUint(raw, 10, 16)
	if err != nil {
		return fallback
	}
	return uint16(port)
}
