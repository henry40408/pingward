package server_test

import (
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/models"
)

func TestCreateProjectAppearsInList(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	res := a.api("POST", "/api/v1/projects", token, obj{"name": "Backups"})
	wantStatus(t, res, 201)
	body := decode[obj](t, res)
	if body["name"] != "Backups" || body["owner_id"] != float64(uid) {
		t.Fatal(res.Body)
	}
	list := decode[[]obj](t, a.api("GET", "/api/v1/projects", token, nil))
	if len(list) != 1 || list[0]["id"] != body["id"] {
		t.Fatalf("list = %v", list)
	}
}

func TestProjectDescriptionIsRawMarkdownAndDefaultsToEmpty(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "alice", false)
	created := a.api("POST", "/api/v1/projects", token, obj{"name": "Backups"})
	wantStatus(t, created, 201)
	body := decode[obj](t, created)
	if body["description"] != "" {
		t.Fatal(created.Body)
	}
	path := fmt.Sprintf("/api/v1/projects/%v", body["id"])
	patched := a.api("PATCH", path, token, obj{"name": "Backups", "description": "Nightly **bold** backups."})
	wantStatus(t, patched, 200)
	if decode[obj](t, patched)["description"] != "Nightly **bold** backups." {
		t.Fatal(patched.Body)
	}
	fetched := a.api("GET", path, token, nil)
	if !strings.Contains(fetched.Body, "**bold**") || strings.Contains(fetched.Body, "<strong>") {
		t.Fatalf("GET must return raw markdown: %s", fetched.Body)
	}
}

func TestCreateProjectRejectsBlankName(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "alice", false)
	res := a.api("POST", "/api/v1/projects", token, obj{"name": "   "})
	wantStatus(t, res, 400)
	if errorCode(t, res) != "bad_request" {
		t.Fatal(res.Body)
	}
}

func TestDurationFieldAcceptsBothIntAndString(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "alice", false)
	for _, tc := range []struct {
		name  string
		value any
		want  float64
	}{{"ints", 90, 90}, {"strs", "5m", 300}} {
		res := a.api("POST", "/api/v1/projects", token, obj{"name": tc.name, "scan_interval_secs": tc.value})
		wantStatus(t, res, 201)
		if decode[obj](t, res)["scan_interval_secs"] != tc.want {
			t.Errorf("%v: %s", tc.value, res.Body)
		}
	}
}

func TestPatchProjectReplacesFields(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	pid := a.newProject(t, uid, "old")
	res := a.api("PATCH", fmt.Sprintf("/api/v1/projects/%d", pid), token, obj{"name": "new", "nag_interval_secs": "1h"})
	wantStatus(t, res, 200)
	body := decode[obj](t, res)
	if body["name"] != "new" || body["nag_interval_secs"] != float64(3600) || body["scan_interval_secs"] != nil {
		t.Fatal(res.Body)
	}
}

func TestDeleteProjectThenGone(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	path := fmt.Sprintf("/api/v1/projects/%d", a.newProject(t, uid, "p"))
	wantStatus(t, a.api("DELETE", path, token, nil), 204)
	wantStatus(t, a.api("GET", path, token, nil), 404)
}

func TestMemberCannotCreateCheckInAnotherUsersProject(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "alice", false)
	bob, _ := a.apiUser(t, "bob", false)
	pid := a.newProject(t, bob, "bobs")
	wantStatus(t, a.api("POST", fmt.Sprintf("/api/v1/projects/%d/checks", pid), token, obj{"name": "x", "period_secs": 60}), 404)
	if checks, _ := a.store.ListChecksForProject(t.Context(), pid); len(checks) != 0 {
		t.Fatalf("a check was created: %v", checks)
	}
}

func TestAdminCrossUserWriteIsAuditedWithTheVerb(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "root", true)
	bob, _ := a.apiUser(t, "bob", false)
	pid := a.newProject(t, bob, "bobs")
	wantStatus(t, a.api("POST", fmt.Sprintf("/api/v1/projects/%d/checks", pid), token, obj{"name": "cross", "period_secs": "1h"}), 201)
	audits, err := a.store.ListAudit(t.Context(), 10)
	if err != nil || len(audits) != 1 {
		t.Fatalf("audits = %v, %v", audits, err)
	}
	got := audits[0]
	if got.Action != "admin.api.access" || *got.Method != "POST" || *got.TargetType != "project" || *got.TargetOwnerID != bob {
		t.Fatalf("audit = %+v", got)
	}
}

func TestCreateCheckPeriodAndRejectBadSchedule(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	path := fmt.Sprintf("/api/v1/projects/%d/checks", a.newProject(t, uid, "p"))
	ok := a.api("POST", path, token, obj{"name": "job", "period_secs": "1h", "grace_secs": "5m"})
	wantStatus(t, ok, 201)
	body := decode[obj](t, ok)
	if body["period_secs"] != float64(3600) || body["grace_secs"] != float64(300) || body["status"] != "new" || body["ping_uuid"] == "" {
		t.Fatal(ok.Body)
	}
	bad := a.api("POST", path, token, obj{"name": "bad", "schedule_kind": "cron"})
	wantStatus(t, bad, 400)
	if errorCode(t, bad) != "bad_request" {
		t.Fatal(bad.Body)
	}
}

func TestCreateCheckIsBoundToExistingProjectChannels(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	pid := a.newProject(t, uid, "p")
	c1 := a.newChannel(t, pid, models.ChannelWebhook, "hook1", `{"url":"http://x"}`)
	c2 := a.newChannel(t, pid, models.ChannelWebhook, "hook2", `{"url":"http://y"}`)
	res := a.api("POST", fmt.Sprintf("/api/v1/projects/%d/checks", pid), token, obj{"name": "job", "period_secs": "1h", "grace_secs": "5m"})
	wantStatus(t, res, 201)
	bound, err := a.store.BoundChannelIDs(t.Context(), int64(decode[obj](t, res)["id"].(float64)))
	if err != nil {
		t.Fatal(err)
	}
	slices.Sort(bound)
	if !slices.Equal(bound, []int64{c1, c2}) {
		t.Fatalf("bound = %v, want [%d %d]", bound, c1, c2)
	}
}

func TestCheckDescriptionIsRawMarkdownAndDefaultsToEmpty(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	created := a.api("POST", fmt.Sprintf("/api/v1/projects/%d/checks", a.newProject(t, uid, "p")), token,
		obj{"name": "job", "period_secs": "1h", "grace_secs": "5m"})
	wantStatus(t, created, 201)
	body := decode[obj](t, created)
	if body["description"] != "" {
		t.Fatal(created.Body)
	}
	path := fmt.Sprintf("/api/v1/checks/%v", body["id"])
	patched := a.api("PATCH", path, token, obj{"name": "job", "description": "Runs **nightly**.", "period_secs": "1h", "grace_secs": "5m"})
	wantStatus(t, patched, 200)
	if decode[obj](t, patched)["description"] != "Runs **nightly**." {
		t.Fatal(patched.Body)
	}
	fetched := a.api("GET", path, token, nil)
	if !strings.Contains(fetched.Body, "**nightly**") || strings.Contains(fetched.Body, "<strong>") {
		t.Fatalf("GET must return raw markdown: %s", fetched.Body)
	}
}

func TestCheckActionsChangeState(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	cid := a.newCheck(t, a.newProject(t, uid, "p"), "job", "uuid-orig")
	action := func(name string) obj {
		res := a.api("POST", fmt.Sprintf("/api/v1/checks/%d/%s", cid, name), token, nil)
		wantStatus(t, res, 200)
		return decode[obj](t, res)
	}
	if action("pause")["status"] != "paused" || action("resume")["status"] != "new" ||
		action("ack")["acknowledged"] != true || action("regenerate")["ping_uuid"] == "uuid-orig" {
		t.Fatal("an action did not change the check")
	}
}

func TestPatchCheckReplacesSchedule(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	cid := a.newCheck(t, a.newProject(t, uid, "p"), "job", "uuid-x")
	res := a.api("PATCH", fmt.Sprintf("/api/v1/checks/%d", cid), token, obj{"name": "renamed", "period_secs": "2h", "grace_secs": 45})
	wantStatus(t, res, 200)
	body := decode[obj](t, res)
	if body["name"] != "renamed" || body["period_secs"] != float64(7200) || body["grace_secs"] != float64(45) || body["ping_uuid"] != "uuid-x" {
		t.Fatal(res.Body)
	}
}

func TestCheckOverrideFieldsRoundTrip(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	created := a.api("POST", fmt.Sprintf("/api/v1/projects/%d/checks", a.newProject(t, uid, "p")), token, obj{
		"name": "job", "period_secs": "1h", "scan_interval_secs": "30s", "max_runtime_secs": "10m", "nag_interval_secs": "2h",
	})
	wantStatus(t, created, 201)
	path := fmt.Sprintf("/api/v1/checks/%v", decode[obj](t, created)["id"])
	for _, text := range []string{created.Body, a.api("GET", path, token, nil).Body} {
		var body obj
		if err := json.Unmarshal([]byte(text), &body); err != nil {
			t.Fatal(err)
		}
		if body["scan_interval_secs"] != float64(30) || body["max_runtime_secs"] != float64(600) || body["nag_interval_secs"] != float64(7200) {
			t.Fatal(text)
		}
	}
	patched := a.api("PATCH", path, token, obj{"name": "job", "period_secs": 60, "max_runtime_secs": 120})
	wantStatus(t, patched, 200)
	body := decode[obj](t, patched)
	if body["max_runtime_secs"] != float64(120) || body["scan_interval_secs"] != nil || body["nag_interval_secs"] != nil {
		t.Fatal(patched.Body)
	}
}

func TestSetCheckChannelsHonorsOnlySameProjectChannels(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	pid := a.newProject(t, uid, "p")
	cid := a.newCheck(t, pid, "job", "uuid-ch")
	ch := a.newChannel(t, pid, models.ChannelWebhook, "hook", `{"url":"https://e.example"}`)
	path := fmt.Sprintf("/api/v1/checks/%d/channels", cid)
	res := a.api("PUT", path, token, obj{"channel_ids": []int64{ch, 9999}})
	wantStatus(t, res, 200)
	if res.Body != fmt.Sprintf(`{"channel_ids":[%d]}`, ch) {
		t.Fatal(res.Body)
	}
	cleared := a.api("PUT", path, token, obj{"channel_ids": []int64{}})
	if cleared.Body != `{"channel_ids":[]}` {
		t.Fatal(cleared.Body)
	}
}

func (a *app) channelConfig(t *testing.T, id int64) (string, obj) {
	t.Helper()
	stored, err := a.store.FindChannel(t.Context(), id)
	if err != nil || stored == nil {
		t.Fatalf("channel %d: %v", id, err)
	}
	var config obj
	if err := json.Unmarshal([]byte(stored.ConfigJSON), &config); err != nil {
		t.Fatal(err)
	}
	return stored.Name, config
}

func TestCreateChannelHidesSecretsThenDelete(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	secret := "https://hooks.example.com/SECRET"
	res := a.api("POST", fmt.Sprintf("/api/v1/projects/%d/channels", a.newProject(t, uid, "p")), token,
		obj{"name": "hook", "kind": "webhook", "webhook_url": secret})
	wantStatus(t, res, 201)
	if strings.Contains(res.Body, secret) || strings.Contains(res.Body, "config_json") {
		t.Fatalf("config leaked: %s", res.Body)
	}
	chid := int64(decode[obj](t, res)["id"].(float64))
	if _, config := a.channelConfig(t, chid); config["url"] != secret {
		t.Fatalf("stored config = %v", config)
	}
	wantStatus(t, a.api("DELETE", fmt.Sprintf("/api/v1/channels/%d", chid), token, nil), 204)
	if stored, _ := a.store.FindChannel(t.Context(), chid); stored != nil {
		t.Fatal("channel survived the delete")
	}
}

func TestPatchChannelRenamesWithoutTouchingTheStoredSecret(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	secret := "https://hooks.example.com/SECRET"
	chid := a.newChannel(t, a.newProject(t, uid, "p"), models.ChannelWebhook, "hook", `{"url":"`+secret+`"}`)
	res := a.api("PATCH", fmt.Sprintf("/api/v1/channels/%d", chid), token, obj{"name": "renamed"})
	wantStatus(t, res, 200)
	if strings.Contains(res.Body, secret) || strings.Contains(res.Body, "config_json") || decode[obj](t, res)["name"] != "renamed" {
		t.Fatal(res.Body)
	}
	if name, config := a.channelConfig(t, chid); name != "renamed" || config["url"] != secret {
		t.Fatalf("stored = %s %v", name, config)
	}
}

func TestPatchChannelRotatesOnlyTheSubmittedCredential(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	chid := a.newChannel(t, a.newProject(t, uid, "p"), models.ChannelTelegram, "tg", `{"token":"OLD-TOKEN","chat_id":"12345"}`)
	wantStatus(t, a.api("PATCH", fmt.Sprintf("/api/v1/channels/%d", chid), token, obj{"telegram_token": "NEW-TOKEN"}), 200)
	name, config := a.channelConfig(t, chid)
	if config["token"] != "NEW-TOKEN" || config["chat_id"] != "12345" || name != "tg" {
		t.Fatalf("stored = %s %v", name, config)
	}
}

func TestPatchChannelIgnoresASubmittedKind(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	chid := a.newChannel(t, a.newProject(t, uid, "p"), models.ChannelWebhook, "hook", `{"url":"https://hooks.example.com/SECRET"}`)
	res := a.api("PATCH", fmt.Sprintf("/api/v1/channels/%d", chid), token, obj{"kind": "slack", "slack_url": "https://hooks.slack.com/services/X"})
	wantStatus(t, res, 200)
	stored, _ := a.store.FindChannel(t.Context(), chid)
	_, config := a.channelConfig(t, chid)
	if decode[obj](t, res)["kind"] != "webhook" || stored.Kind != models.ChannelWebhook || config["url"] != "https://hooks.example.com/SECRET" {
		t.Fatalf("kind changed: %s %v", res.Body, config)
	}
}

func TestPatchChannelClearsTheNtfyTokenOnRequest(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	chid := a.newChannel(t, a.newProject(t, uid, "p"), models.ChannelNtfy, "n", `{"base_url":"https://ntfy.sh","topic":"t","token":"TOK"}`)
	path := fmt.Sprintf("/api/v1/channels/%d", chid)
	wantStatus(t, a.api("PATCH", path, token, obj{"ntfy_token": ""}), 200)
	if _, config := a.channelConfig(t, chid); config["token"] != "TOK" {
		t.Fatalf("a blank token must keep the stored one: %v", config)
	}
	wantStatus(t, a.api("PATCH", path, token, obj{"ntfy_token_clear": true}), 200)
	if _, config := a.channelConfig(t, chid); config["token"] != "" || config["topic"] != "t" {
		t.Fatalf("clear touched more than the token: %v", config)
	}
}

func TestPatchChannelRejectsBlankingARequiredCredential(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	chid := a.newChannel(t, a.newProject(t, uid, "p"), models.ChannelWebhook, "hook", `{}`)
	wantStatus(t, a.api("PATCH", fmt.Sprintf("/api/v1/channels/%d", chid), token, obj{"name": "renamed"}), 400)
	if name, _ := a.channelConfig(t, chid); name != "hook" {
		t.Fatalf("a rejected patch applied the name: %s", name)
	}
}

func TestCreateChannelRejectsMissingRequiredField(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	res := a.api("POST", fmt.Sprintf("/api/v1/projects/%d/channels", a.newProject(t, uid, "p")), token, obj{"name": "hook", "kind": "webhook"})
	wantStatus(t, res, 400)
}

func TestMalformedJSONBodyIsA400Envelope(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "alice", false)
	res := a.api("POST", "/api/v1/projects", token, "{ not valid json ")
	wantStatus(t, res, 400)
	if errorCode(t, res) != "bad_request" {
		t.Fatal(res.Body)
	}
}
