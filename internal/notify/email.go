package notify

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/tls"
	"encoding/hex"
	"fmt"
	"mime"
	"mime/quotedprintable"
	"net"
	"net/mail"
	"net/smtp"
	"strconv"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/config"
)

// smtpIOTimeout bounds every read and write, so each SMTP command gets its own
// 60 second budget.
const smtpIOTimeout = 60 * time.Second

type emailNotifier struct {
	smtp *config.SMTPConfig
	to   string
}

// NewEmail sends through the instance SMTP relay. With none configured, Send
// errors so the failure is recorded rather than the alert silently dropped.
func NewEmail(smtpConfig *config.SMTPConfig, to string) Notifier {
	return &emailNotifier{smtp: smtpConfig, to: to}
}

type emailMessage struct {
	from, to *mail.Address
	data     []byte
}

// buildEmail turns a malformed address into an Error. Address parsing rejects
// CR and LF, and the subject is Q-encoded, so no field can inject a header.
func buildEmail(from, to string, ev Event) (emailMessage, error) {
	fromAddress, err := mail.ParseAddress(from)
	if err != nil {
		return emailMessage{}, newError("invalid from address: %v", err)
	}
	toAddress, err := mail.ParseAddress(to)
	if err != nil {
		return emailMessage{}, newError("invalid recipient address: %v", err)
	}
	messageID, err := newMessageID(fromAddress.Address)
	if err != nil {
		return emailMessage{}, newError("failed to build email: %v", err)
	}
	var body bytes.Buffer
	writer := quotedprintable.NewWriter(&body)
	if _, err := writer.Write([]byte(EventText(ev))); err != nil {
		return emailMessage{}, newError("failed to build email: %v", err)
	}
	if err := writer.Close(); err != nil {
		return emailMessage{}, newError("failed to build email: %v", err)
	}
	var data bytes.Buffer
	fmt.Fprintf(&data, "From: %s\r\n", fromAddress.String())
	fmt.Fprintf(&data, "To: %s\r\n", toAddress.String())
	fmt.Fprintf(&data, "Subject: %s\r\n", mime.QEncoding.Encode("utf-8", EventTitle(ev)))
	fmt.Fprintf(&data, "Date: %s\r\n", time.Now().UTC().Format(time.RFC1123Z))
	fmt.Fprintf(&data, "Message-ID: %s\r\n", messageID)
	data.WriteString("MIME-Version: 1.0\r\n")
	data.WriteString("Content-Type: text/plain; charset=utf-8\r\n")
	data.WriteString("Content-Transfer-Encoding: quoted-printable\r\n\r\n")
	data.Write(body.Bytes())
	return emailMessage{from: fromAddress, to: toAddress, data: data.Bytes()}, nil
}

func newMessageID(fromAddress string) (string, error) {
	random := make([]byte, 16)
	if _, err := rand.Read(random); err != nil {
		return "", err
	}
	domain := "localhost"
	if at := strings.LastIndexByte(fromAddress, '@'); at >= 0 {
		domain = fromAddress[at+1:]
	}
	return fmt.Sprintf("<%s@%s>", hex.EncodeToString(random), domain), nil
}

func (n *emailNotifier) Send(ctx context.Context, ev Event) error {
	if n.smtp == nil {
		return &Error{Message: "instance SMTP not configured"}
	}
	message, err := buildEmail(n.smtp.From, n.to, ev)
	if err != nil {
		return err
	}
	if err := sendSMTP(ctx, *n.smtp, message); err != nil {
		return newError("smtp send failed: %v", err)
	}
	return nil
}

// sendSMTP opens a fresh connection per message.
func sendSMTP(ctx context.Context, cfg config.SMTPConfig, message emailMessage) error {
	client, err := connectSMTP(ctx, cfg)
	if err != nil {
		return err
	}
	defer func() { _ = client.Close() }()
	if cfg.Username != nil && cfg.Password != nil {
		if ok, _ := client.Extension("AUTH"); !ok {
			return fmt.Errorf("server does not offer authentication")
		}
		if err := client.Auth(plainAuth{username: *cfg.Username, password: *cfg.Password}); err != nil {
			return err
		}
	}
	if err := client.Mail(message.from.Address); err != nil {
		return err
	}
	if err := client.Rcpt(message.to.Address); err != nil {
		return err
	}
	data, err := client.Data()
	if err != nil {
		return err
	}
	if _, err := data.Write(message.data); err != nil {
		return err
	}
	if err := data.Close(); err != nil {
		return err
	}
	return client.Quit()
}

// connectSMTP honours the TLS mode. STARTTLS must succeed: it never falls back
// to plain text.
func connectSMTP(ctx context.Context, cfg config.SMTPConfig) (*smtp.Client, error) {
	dialer := net.Dialer{Timeout: smtpIOTimeout}
	address := net.JoinHostPort(cfg.Host, strconv.Itoa(int(cfg.Port)))
	rawConn, err := dialer.DialContext(ctx, "tcp", address)
	if err != nil {
		return nil, err
	}
	var conn net.Conn = &deadlineConn{Conn: rawConn}
	tlsConfig := &tls.Config{ServerName: cfg.Host, MinVersion: tls.VersionTLS12}
	if cfg.TLS == config.TLSImplicit {
		conn = &deadlineConn{Conn: tls.Client(conn, tlsConfig)}
	}
	client, err := smtp.NewClient(conn, cfg.Host)
	if err != nil {
		_ = conn.Close()
		return nil, err
	}
	if cfg.TLS != config.TLSStartTLS {
		return client, nil
	}
	if ok, _ := client.Extension("STARTTLS"); !ok {
		_ = client.Close()
		return nil, fmt.Errorf("server does not support STARTTLS")
	}
	if err := client.StartTLS(tlsConfig); err != nil {
		_ = client.Close()
		return nil, err
	}
	return client, nil
}

// deadlineConn renews the deadline before every read and write.
type deadlineConn struct{ net.Conn }

func (c *deadlineConn) Read(p []byte) (int, error) {
	_ = c.SetDeadline(time.Now().Add(smtpIOTimeout))
	return c.Conn.Read(p)
}

func (c *deadlineConn) Write(p []byte) (int, error) {
	_ = c.SetDeadline(time.Now().Add(smtpIOTimeout))
	return c.Conn.Write(p)
}

// plainAuth sends the password on any connection. net/smtp's PlainAuth refuses
// unencrypted links, but the operator chose PINGWARD_SMTP_TLS=none on purpose
// and startup already warns about it.
type plainAuth struct{ username, password string }

func (a plainAuth) Start(*smtp.ServerInfo) (string, []byte, error) {
	return "PLAIN", []byte("\x00" + a.username + "\x00" + a.password), nil
}

func (a plainAuth) Next(_ []byte, more bool) ([]byte, error) {
	if more {
		return nil, fmt.Errorf("unexpected server challenge")
	}
	return nil, nil
}
