package notify

import (
	"bufio"
	"encoding/base64"
	"encoding/json"
	"net"
	"net/http"
	"os"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/models"
)

// Real SMTP delivery to Mailpit, checked via its REST API. Skipped unless
// PINGWARD_TEST_SMTP_HOST is set (CI sets it).
func TestEmailChannelDeliversOverSMTPToRelay(t *testing.T) {
	host := os.Getenv("PINGWARD_TEST_SMTP_HOST")
	if host == "" {
		t.Skip("PINGWARD_TEST_SMTP_HOST unset")
	}
	port := uint16(1025)
	if parsed, err := strconv.ParseUint(os.Getenv("PINGWARD_TEST_SMTP_PORT"), 10, 16); err == nil {
		port = uint16(parsed)
	}
	api := os.Getenv("PINGWARD_TEST_MAILPIT_API")
	if api == "" {
		t.Fatal("PINGWARD_TEST_MAILPIT_API must be set when PINGWARD_TEST_SMTP_HOST is")
	}
	request, _ := http.NewRequest(http.MethodDelete, api+"/api/v1/messages", nil)
	if _, err := http.DefaultClient.Do(request); err != nil {
		t.Fatalf("clear mailpit mailbox: %v", err)
	}

	smtp := &config.SMTPConfig{Host: host, Port: port, From: "alerts@example.com", TLS: config.TLSNone}
	channel := channelWith(models.ChannelEmail, `{"to":"ops@example.com"}`)
	notifier, ok := NotifierFor(channel, smtp)
	if !ok {
		t.Fatal("no email notifier for configured SMTP")
	}
	if err := notifier.Send(bg, simpleEvent("backup", models.EventDown)); err != nil {
		t.Fatal(err)
	}

	resp, err := http.Get(api + "/api/v1/messages")
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	var messages struct {
		Total    int
		Messages []struct {
			From    struct{ Address string }
			To      []struct{ Address string }
			Subject string
		}
	}
	if err := json.NewDecoder(resp.Body).Decode(&messages); err != nil || messages.Total != 1 {
		t.Fatalf("exactly one message should be delivered: %+v %v", messages, err)
	}
	message := messages.Messages[0]
	if message.From.Address != "alerts@example.com" || message.To[0].Address != "ops@example.com" ||
		!strings.Contains(message.Subject, "pingward") || !strings.Contains(message.Subject, "backup") {
		t.Fatalf("%+v", message)
	}
}

// fakeSMTP is a plain-text SMTP server that records one session.
type fakeSMTP struct {
	addr  net.Addr
	lines chan string
}

func startFakeSMTP(t *testing.T, ehloExtensions ...string) *fakeSMTP {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { listener.Close() })
	server := &fakeSMTP{addr: listener.Addr(), lines: make(chan string, 64)}
	go func() {
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		defer conn.Close()
		server.serve(conn, ehloExtensions)
	}()
	return server
}

func (s *fakeSMTP) serve(conn net.Conn, extensions []string) {
	_ = conn.SetDeadline(time.Now().Add(10 * time.Second))
	reader := bufio.NewReader(conn)
	reply := func(text string) { _, _ = conn.Write([]byte(text + "\r\n")) }
	reply("220 fake ESMTP")
	inData := false
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			return
		}
		line = strings.TrimRight(line, "\r\n")
		s.lines <- line
		switch {
		case inData && line == ".":
			inData = false
			reply("250 queued")
		case inData:
		case strings.HasPrefix(line, "EHLO"):
			reply("250-fake")
			for _, extension := range extensions {
				reply("250-" + extension)
			}
			reply("250 OK")
		case strings.HasPrefix(line, "AUTH PLAIN"):
			reply("235 ok")
		case line == "DATA":
			inData = true
			reply("354 go")
		case line == "QUIT":
			reply("221 bye")
			return
		default:
			reply("250 OK")
		}
	}
}

func (s *fakeSMTP) config(mode config.TLSMode, username, password *string) *config.SMTPConfig {
	address := s.addr.(*net.TCPAddr)
	return &config.SMTPConfig{Host: "127.0.0.1", Port: uint16(address.Port), From: "alerts@example.com",
		TLS: mode, Username: username, Password: password}
}

func (s *fakeSMTP) transcript() string {
	var lines []string
	for {
		select {
		case line := <-s.lines:
			lines = append(lines, line)
		default:
			return strings.Join(lines, "\n")
		}
	}
}

func TestEmailInPlainModeSendsTheMessageAndCredentials(t *testing.T) {
	server := startFakeSMTP(t, "AUTH PLAIN")
	smtp := server.config(config.TLSNone, ptr("user"), ptr("secret"))
	ev := simpleEvent("backup", models.EventDown)
	if err := NewEmail(smtp, "ops@example.com").Send(bg, ev); err != nil {
		t.Fatal(err)
	}
	transcript := server.transcript()
	credentials := base64.StdEncoding.EncodeToString([]byte("\x00user\x00secret"))
	requireContains(t, transcript, "AUTH PLAIN "+credentials)
	requireContains(t, transcript, "MAIL FROM:<alerts@example.com>")
	requireContains(t, transcript, "RCPT TO:<ops@example.com>")
	requireContains(t, transcript, "Subject: pingward: backup is DOWN")
}

func TestEmailStartTLSNeverFallsBackToPlainText(t *testing.T) {
	server := startFakeSMTP(t, "AUTH PLAIN")
	smtp := server.config(config.TLSStartTLS, ptr("user"), ptr("secret"))
	err := NewEmail(smtp, "ops@example.com").Send(bg, simpleEvent("backup", models.EventDown))
	if err == nil || !strings.Contains(err.Error(), "STARTTLS") {
		t.Fatalf("%v", err)
	}
	transcript := server.transcript()
	requireAbsent(t, transcript, "AUTH")
	requireAbsent(t, transcript, "MAIL FROM")
}

func TestEmailCredentialsNeedAServerThatOffersAuth(t *testing.T) {
	server := startFakeSMTP(t)
	smtp := server.config(config.TLSNone, ptr("user"), ptr("secret"))
	if err := NewEmail(smtp, "ops@example.com").Send(bg, simpleEvent("backup", models.EventDown)); err == nil {
		t.Fatal("want error")
	}
	requireAbsent(t, server.transcript(), "MAIL FROM")
}

func TestEmailStartTLSFailsWhenTheServerRefusesTheUpgrade(t *testing.T) {
	// The fake answers STARTTLS with 250 instead of 220.
	server := startFakeSMTP(t, "STARTTLS", "AUTH PLAIN")
	smtp := server.config(config.TLSStartTLS, ptr("user"), ptr("secret"))
	if err := NewEmail(smtp, "ops@example.com").Send(bg, simpleEvent("backup", models.EventDown)); err == nil {
		t.Fatal("want error")
	}
	transcript := server.transcript()
	requireContains(t, transcript, "STARTTLS")
	requireAbsent(t, transcript, "AUTH PLAIN")
	requireAbsent(t, transcript, "MAIL FROM")
}

func TestEmailImplicitTLSFailsAgainstAPlainServer(t *testing.T) {
	server := startFakeSMTP(t)
	smtp := server.config(config.TLSImplicit, nil, nil)
	if err := NewEmail(smtp, "ops@example.com").Send(bg, simpleEvent("backup", models.EventDown)); err == nil {
		t.Fatal("want error")
	}
	requireAbsent(t, server.transcript(), "MAIL FROM")
}

func TestEmailFailsWhenNothingListens(t *testing.T) {
	server := startFakeSMTP(t)
	smtp := server.config(config.TLSNone, nil, nil)
	smtp.Port = 1 // reserved port: nothing listens there
	if err := NewEmail(smtp, "ops@example.com").Send(bg, simpleEvent("backup", models.EventDown)); err == nil {
		t.Fatal("want error")
	}
}
