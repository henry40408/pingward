package server_test

import (
	"encoding/json"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/api"
	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
	"github.com/henry40408/pingward/internal/web"
)

// apiUser creates a user with an API key and returns the id and the bearer token.
func (a *app) apiUser(t *testing.T, username string, isAdmin bool) (int64, string) {
	t.Helper()
	id := a.addUser(t, username, isAdmin)
	key := apikey.Generate()
	_, err := a.store.InsertAPIKey(t.Context(), store.NewAPIKey{UserID: id, Name: "k", TokenHash: key.Hash, Prefix: key.Prefix}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id, key.Full
}

// api sends a JSON request with a bearer token; body is marshalled unless it is a string or nil.
func (a *app) api(method, path, token string, body any) *testutil.Response {
	headers := http.Header{}
	if token != "" {
		headers.Set("Authorization", "Bearer "+token)
	}
	var reader *strings.Reader
	switch typed := body.(type) {
	case nil:
		return a.client.Do(method, path, nil, headers)
	case string:
		reader = strings.NewReader(typed)
	default:
		encoded, err := json.Marshal(typed)
		if err != nil {
			a.client.T.Fatal(err)
		}
		reader = strings.NewReader(string(encoded))
	}
	headers.Set("Content-Type", "application/json")
	return a.client.Do(method, path, reader, headers)
}

type obj = map[string]any

func decode[T any](t *testing.T, res *testutil.Response) T {
	t.Helper()
	var value T
	if err := json.Unmarshal([]byte(res.Body), &value); err != nil {
		t.Fatalf("body is not the expected JSON: %v: %s", err, res.Body)
	}
	return value
}

func (a *app) newProject(t *testing.T, ownerID int64, name string) int64 {
	t.Helper()
	id, err := a.store.CreateProject(t.Context(), ownerID, store.ProjectSettings{Name: name}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func (a *app) newCheck(t *testing.T, projectID int64, name, uuid string) int64 {
	t.Helper()
	period := int64(60)
	id, err := a.store.CreateCheck(t.Context(), store.NewCheck{
		ProjectID: projectID, Name: name, PingUUID: uuid, Kind: models.SchedulePeriod,
		PeriodSecs: &period, GraceSecs: 30, Timezone: "UTC",
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func (a *app) newChannel(t *testing.T, projectID int64, kind models.ChannelKind, name, config string) int64 {
	t.Helper()
	id, err := a.store.CreateChannel(t.Context(), store.NewChannel{ProjectID: projectID, Kind: kind, Name: name, ConfigJSON: config}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func idPath(format string, id int64) string {
	return strings.Replace(format, "{id}", itoa(id), 1)
}

func errorCode(t *testing.T, res *testutil.Response) string {
	t.Helper()
	return decode[map[string]obj](t, res)["error"]["code"].(string)
}

func apiRoutes(a *app) []web.Route { return api.New(a.deps).Routes() }
