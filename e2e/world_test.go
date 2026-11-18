package e2e

import (
	"context"
	"errors"
	"fmt"
	"net/url"
	"regexp"
	"strings"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// world is the state shared by the steps of one scenario.
type world struct {
	shard   *harness.Shard
	server  *harness.Server
	browser *harness.Browser

	mockWebhook *harness.MockWebhook

	projectURL string
	projectID  int64
	checkID    int64
	pingURL    string
	webhookURL string
	// status is the last out-of-band HTTP status.
	status int
	// pingStatus is the status of the last ping.
	pingStatus int
}

// initializeScenario runs once per scenario, so every scenario gets its own
// world through the closure.
func initializeScenario(sc *godog.ScenarioContext, shard *harness.Shard) {
	w := &world{shard: shard}
	sc.Before(w.start)
	sc.After(w.cleanUp)
	registerAllSteps(sc, w)
}

func (w *world) start(ctx context.Context, scenario *godog.Scenario) (context.Context, error) {
	if w.shard != nil && !w.shard.Includes(shardKey(scenario)) {
		return ctx, godog.ErrSkip
	}
	tags := make([]string, 0, len(scenario.Tags))
	for _, tag := range scenario.Tags {
		tags = append(tags, strings.TrimPrefix(tag.Name, "@"))
	}
	server, err := harness.Start(harness.OptionsFromTags(tags))
	if err != nil {
		return ctx, err
	}
	w.server = server
	scripting := harness.ScriptsOn
	for _, tag := range tags {
		if tag == "nojs" {
			scripting = harness.ScriptsOff
		}
	}
	w.browser, err = harness.OpenWith(scripting)
	return ctx, err
}

// shardKey tells apart the rows an outline expands into by their AST node ids.
func shardKey(scenario *godog.Scenario) string {
	return fmt.Sprintf("%s\x00%s\x00%s", scenario.Uri, scenario.Name, strings.Join(scenario.AstNodeIds, ","))
}

func (w *world) cleanUp(ctx context.Context, _ *godog.Scenario, _ error) (context.Context, error) {
	if w.browser != nil {
		w.browser.Close()
	}
	if w.mockWebhook != nil {
		w.mockWebhook.Close()
	}
	if w.server != nil {
		w.server.Stop()
	}
	return ctx, nil
}

func (w *world) api() *harness.API { return harness.NewAPI(w.server) }

func (w *world) goTo(path string) error {
	return w.browser.Goto(w.server.BaseURL + path)
}

// path is the current URL's path and query.
func (w *world) path() (string, error) { return w.browser.PathAndQuery() }

// expectPath waits for the path and query to equal expected.
func (w *world) expectPath(expected string) error {
	return harness.EventuallyEq("URL is "+expected, expected, w.path)
}

// expectPathMatching waits for the path and query to match the regex.
func (w *world) expectPathMatching(pattern string) error {
	expression, err := regexp.Compile(pattern)
	if err != nil {
		return err
	}
	return harness.Eventually("URL matches "+pattern, func() (bool, error) {
		current, err := w.path()
		return expression.MatchString(current), err
	})
}

// fetchStatus is the status of a fetch issued by the page. It carries the
// session cookie but no CSRF token, so authz scenarios see the ownership 404
// and CSRF scenarios the missing token.
func (w *world) fetchStatus(method, path string) (int, error) {
	script := fmt.Sprintf(`(async () => {
		const init = { method: %s, credentials: 'same-origin' };
		if (init.method !== 'GET') { init.body = new URLSearchParams(); }
		try { return (await fetch(%s, init)).status; } catch (e) { return 0; }
	})()`, harness.JSLiteral(method), harness.JSLiteral(w.server.BaseURL+path))
	status, err := harness.EvalAsync[int](w.browser, script)
	if err != nil {
		return 0, err
	}
	if status == 0 {
		return 0, fmt.Errorf("the request to %s never completed", path)
	}
	return status, nil
}

func (w *world) requireProject() (string, error) {
	if w.projectURL == "" {
		return "", errors.New("no project: no step created one for this scenario")
	}
	return w.projectURL, nil
}

// mock starts the scenario's webhook receiver on first use, since most
// scenarios never deliver anything.
func (w *world) mock() *harness.MockWebhook {
	if w.mockWebhook == nil {
		w.mockWebhook = harness.StartMockWebhook()
		w.webhookURL = w.mockWebhook.URL()
	}
	return w.mockWebhook
}

// postFormAsUser POSTs a form with the browser's cookies (the caller supplies
// _csrf) and follows no redirects.
func (w *world) postFormAsUser(path string, form url.Values) (int, error) {
	cookies, err := w.browser.CookieHeader()
	if err != nil {
		return 0, err
	}
	return w.api().PostFormAs(cookies, path, form)
}

func (w *world) requireWebhookURL() (string, error) {
	if w.webhookURL == "" {
		return "", errors.New("no webhook URL: no step started the mock receiver")
	}
	return w.webhookURL, nil
}
