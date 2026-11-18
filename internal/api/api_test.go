package api

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"slices"
	"strings"
	"testing"
	"time"
)

func TestOpenAPIMatchesRoutes(t *testing.T) {
	var document struct {
		Paths map[string]map[string]json.RawMessage `json:"paths"`
	}
	if err := json.Unmarshal(openAPIJSON, &document); err != nil {
		t.Fatal(err)
	}
	var documented, routed []string
	for path, methods := range document.Paths {
		for method := range methods {
			documented = append(documented, strings.ToUpper(method)+" "+path)
		}
	}
	for _, route := range New(nil).Routes() {
		routed = append(routed, route.Method+" "+route.Pattern)
	}
	slices.Sort(documented)
	slices.Sort(routed)
	if !slices.Equal(documented, routed) {
		t.Fatalf("openapi.json and Routes() differ\nopenapi: %v\nroutes:  %v", documented, routed)
	}
}

func TestEmbeddedDocumentsEqualTheRustCaptures(t *testing.T) {
	spec, err := os.ReadFile("testdata/openapi.json")
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(spec, openAPIJSON) {
		t.Error("embedded openapi.json differs from testdata/openapi.json")
	}
	page, err := os.ReadFile("testdata/docs.html")
	if err != nil {
		t.Fatal(err)
	}
	if got := referencePageHead + string(openAPIJSON) + referencePageTail; got != string(page) {
		t.Error("the reference page differs from testdata/docs.html")
	}
}

func TestTimeFormatIsChronoAutoSi(t *testing.T) {
	base := time.Date(2026, 11, 17, 15, 25, 37, 0, time.FixedZone("x", 8*3600))
	for nanos, want := range map[int]string{
		0:           `"2026-11-17T07:25:37Z"`,
		120_000_000: `"2026-11-17T07:25:37.120Z"`,
		123_450_000: `"2026-11-17T07:25:37.123450Z"`,
		403_017_343: `"2026-11-17T07:25:37.403017343Z"`,
		1_000:       `"2026-11-17T07:25:37.000001Z"`,
	} {
		got, err := Time(base.Add(time.Duration(nanos))).MarshalJSON()
		if err != nil || string(got) != want {
			t.Errorf("%d ns: got %s, want %s", nanos, got, want)
		}
	}
}

func TestDurationAcceptsAnIntegerToo(t *testing.T) {
	request := httptest.NewRequest("POST", "/", strings.NewReader(`{"secs":90}`))
	request.Header.Set("Content-Type", "application/json")
	f, _ := readFields(httptest.NewRecorder(), request)
	if got := f.duration("secs", ""); got != "90" || f.refusal() != "" {
		t.Errorf("got %q, %q", got, f.refusal())
	}
}

func TestPageLimitIsClampedAndBeforeWinsOverAfter(t *testing.T) {
	limit := func(n int64) *int64 { return &n }
	for _, tc := range []struct {
		given *int64
		want  int64
	}{{nil, 20}, {limit(0), 1}, {limit(-5), 1}, {limit(50), 50}, {limit(1000), 100}} {
		if got := pageParamsOf(nil, nil, tc.given).limit; got != tc.want {
			t.Errorf("limit %v: got %d, want %d", tc.given, got, tc.want)
		}
	}
	before, after := int64(5), int64(9)
	if pageParamsOf(&before, &after, nil) != pageParamsOf(&before, nil, nil) {
		t.Error("before must win over after")
	}
}

func TestReadFieldsRefusesWhatSerdeRefuses(t *testing.T) {
	for name, tc := range map[string]struct{ contentType, body string }{
		"no content type": {"", `{"name":"x"}`},
		"not JSON":        {"application/json", `{ nope`},
		"an array":        {"application/json", `[]`},
		"null":            {"application/json", `null`},
	} {
		request := httptest.NewRequest("POST", "/", strings.NewReader(tc.body))
		if tc.contentType != "" {
			request.Header.Set("Content-Type", tc.contentType)
		}
		recorder := httptest.NewRecorder()
		if _, ok := readFields(recorder, request); ok || recorder.Code != http.StatusBadRequest {
			t.Errorf("%s: ok=%v status=%d", name, ok, recorder.Code)
		}
	}
}

func TestFieldsRejectNullForRequiredAndWrongTypes(t *testing.T) {
	read := func(body string) *fields {
		request := httptest.NewRequest("POST", "/", strings.NewReader(body))
		request.Header.Set("Content-Type", "application/json")
		f, ok := readFields(httptest.NewRecorder(), request)
		if !ok {
			t.Fatalf("body %s refused", body)
		}
		return f
	}
	for _, body := range []string{`{}`, `{"name":null}`, `{"name":5}`} {
		f := read(body)
		f.required("name")
		if f.refusal() == "" {
			t.Errorf("%s: name accepted", body)
		}
	}
	f := read(`{"on":null}`)
	f.flag("on")
	if f.refusal() == "" {
		t.Error("null accepted for a plain bool")
	}
	f = read(`{"d":null,"name":"x"}`)
	if f.optionalText("d"); f.required("name") != "x" || f.refusal() != "" {
		t.Errorf("optional null refused: %q", f.refusal())
	}
	f = read(`{"secs":"5m"}`)
	if f.duration("secs", "") != "5m" || f.refusal() != "" {
		t.Errorf("duration string refused: %q", f.refusal())
	}
}

func readFieldsFrom(t *testing.T, body string) *fields {
	t.Helper()
	request := httptest.NewRequest("POST", "/", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	f, ok := readFields(httptest.NewRecorder(), request)
	if !ok {
		t.Fatalf("body %s refused", body)
	}
	return f
}

func TestSyntaxErrorNamesTheJSONProblem(t *testing.T) {
	got := syntaxError([]byte(`{ nope`))
	if !strings.Contains(got, "invalid character") {
		t.Fatalf("got %q, want the decoder's message", got)
	}
	if got := syntaxError([]byte(`{}`)); got != "invalid JSON" {
		t.Fatalf("valid JSON: got %q", got)
	}
}

func TestReadFieldsReportsTheSyntaxErrorInTheResponse(t *testing.T) {
	request := httptest.NewRequest("POST", "/", strings.NewReader(`{ nope`))
	request.Header.Set("Content-Type", "application/json")
	recorder := httptest.NewRecorder()
	readFields(recorder, request)
	if body := recorder.Body.String(); !strings.Contains(body, "invalid character") {
		t.Fatalf("body = %s", body)
	}
}

func TestOptionalTextReadsAPresentValueAndSkipsAfterAnError(t *testing.T) {
	f := readFieldsFrom(t, `{"d":"hello"}`)
	if got := f.optionalText("d"); got != "hello" || f.refusal() != "" {
		t.Fatalf("got %q, refusal %q", got, f.refusal())
	}
	f = readFieldsFrom(t, `{"d":"hello"}`)
	f.required("missing")
	if got := f.optionalText("d"); got != "" {
		t.Fatalf("after an error got %q, want empty", got)
	}
}

func TestFlagAcceptsBooleansAndRefusesOtherTypes(t *testing.T) {
	for body, want := range map[string]bool{`{"on":true}`: true, `{"on":false}`: false, `{}`: false} {
		f := readFieldsFrom(t, body)
		if got := f.flag("on"); got != want || f.refusal() != "" {
			t.Errorf("%s: got %v, refusal %q", body, got, f.refusal())
		}
	}
	for _, body := range []string{`{"on":"yes"}`, `{"on":1}`, `{"on":null}`} {
		f := readFieldsFrom(t, body)
		f.flag("on")
		if f.refusal() == "" {
			t.Errorf("%s: accepted", body)
		}
	}
}
