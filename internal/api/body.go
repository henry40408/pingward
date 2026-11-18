package api

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"mime"
	"net/http"
	"strconv"
	"strings"
)

// maxBodyBytes is axum's default body limit, which the Rust build relied on.
const maxBodyBytes = 2 << 20

// fields reads the members of a JSON object body by exact name, as serde does
// (Go's own decoder matches names case-insensitively and takes null for ""). The first
// problem sticks; later reads are no-ops, so a caller checks err once.
type fields struct {
	raw map[string]json.RawMessage
	err error
}

// readFields answers 400 itself when the body is not a JSON object. Every
// refusal is a 400 in the error envelope, as the Rust extractor made it.
func readFields(w http.ResponseWriter, r *http.Request) (*fields, bool) {
	if !isJSONContentType(r.Header.Get("Content-Type")) {
		badRequest(w, "Expected request with `Content-Type: application/json`")
		return nil, false
	}
	body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, maxBodyBytes))
	if err != nil {
		badRequest(w, "Failed to buffer the request body: "+err.Error())
		return nil, false
	}
	if !json.Valid(body) {
		badRequest(w, "Failed to parse the request body as JSON: "+syntaxError(body))
		return nil, false
	}
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(body, &raw); err != nil || raw == nil {
		badRequest(w, "Failed to deserialize the JSON body into the target type: expected a JSON object")
		return nil, false
	}
	return &fields{raw: raw}, true
}

// isJSONContentType accepts application/json and application/*+json.
func isJSONContentType(header string) bool {
	mediaType, _, err := mime.ParseMediaType(header)
	if err != nil {
		return false
	}
	return mediaType == "application/json" ||
		(strings.HasPrefix(mediaType, "application/") && strings.HasSuffix(mediaType, "+json"))
}

func syntaxError(body []byte) string {
	var discard any
	if err := json.Unmarshal(body, &discard); err != nil {
		return err.Error()
	}
	return "invalid JSON"
}

// refusal is the 400 message once a field read failed; "" while all is well.
func (f *fields) refusal() string {
	if f.err == nil {
		return ""
	}
	return "Failed to deserialize the JSON body into the target type: " + f.err.Error()
}

func (f *fields) fail(format string, args ...any) {
	if f.err == nil {
		f.err = fmt.Errorf(format, args...)
	}
}

// lookup returns the member and whether it is present and not null.
func (f *fields) lookup(name string) (json.RawMessage, bool) {
	value, present := f.raw[name]
	if !present || bytes.Equal(value, []byte("null")) {
		return nil, false
	}
	return value, true
}

func (f *fields) decodeString(name string, value json.RawMessage) string {
	var text string
	if err := json.Unmarshal(value, &text); err != nil || bytes.Equal(value, []byte("null")) {
		f.fail("invalid type for `%s`: expected a string", name)
		return ""
	}
	return text
}

// required is a String member with no default.
func (f *fields) required(name string) string {
	if f.err != nil {
		return ""
	}
	value, present := f.raw[name]
	if !present {
		f.fail("missing field `%s`", name)
		return ""
	}
	return f.decodeString(name, value)
}

// text is a String member that falls back when absent; null is refused.
func (f *fields) text(name, fallback string) string {
	if f.err != nil {
		return ""
	}
	value, present := f.raw[name]
	if !present {
		return fallback
	}
	return f.decodeString(name, value)
}

// optionalText is an Option<String> member: absent and null both read as "".
func (f *fields) optionalText(name string) string {
	if f.err != nil {
		return ""
	}
	value, present := f.lookup(name)
	if !present {
		return ""
	}
	return f.decodeString(name, value)
}

// duration is an Option<seconds or duration string> member, read as the form
// text the validators take; absent and null read as fallback.
func (f *fields) duration(name, fallback string) string {
	if f.err != nil {
		return ""
	}
	value, present := f.lookup(name)
	if !present {
		return fallback
	}
	var text string
	if json.Unmarshal(value, &text) == nil {
		return text
	}
	var seconds int64
	if json.Unmarshal(value, &seconds) != nil {
		f.fail("invalid type for `%s`: expected seconds or a duration string", name)
		return ""
	}
	return strconv.FormatInt(seconds, 10)
}

// flag is a bool member that is false when absent.
func (f *fields) flag(name string) bool {
	if f.err != nil {
		return false
	}
	value, present := f.raw[name]
	if !present {
		return false
	}
	var flag bool
	if err := json.Unmarshal(value, &flag); err != nil || bytes.Equal(value, []byte("null")) {
		f.fail("invalid type for `%s`: expected a boolean", name)
	}
	return flag
}

// ids is a list of integers that is empty when absent.
func (f *fields) ids(name string) []int64 {
	if f.err != nil {
		return nil
	}
	value, present := f.raw[name]
	if !present {
		return nil
	}
	var ids []int64
	if err := json.Unmarshal(value, &ids); err != nil || ids == nil {
		f.fail("invalid type for `%s`: expected a list of integers", name)
		return nil
	}
	return ids
}
