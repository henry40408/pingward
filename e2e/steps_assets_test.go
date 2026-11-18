package e2e

import (
	"fmt"
	"regexp"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

var footerVersion = regexp.MustCompile(`^pingward \S+$`)

type assetProbe struct {
	Status int     `json:"status"`
	Error  *string `json:"error"`
}

// registerAssetsSteps covers the static assets the templates reference.
func registerAssetsSteps(sc *godog.ScenarioContext, w *world) {
	// The version may be a tag, tag+distance or bare SHA, so only the
	// `pingward <token>` shape is asserted.
	sc.Step(`^the footer shows the build version$`, func() error {
		if err := w.browser.ExpectVisible("app-version"); err != nil {
			return err
		}
		version, err := w.browser.TextOf("app-version")
		if err != nil {
			return err
		}
		if !footerVersion.MatchString(version) {
			return fmt.Errorf("the footer reads %q", version)
		}
		return nil
	})
	// DOMParser signals failure with a <parsererror> element, not by throwing.
	sc.Step(`^`+quoted+` is well-formed XML$`, func(asset string) error {
		asset = unquote(asset)
		probe, err := harness.EvalAsync[assetProbe](w.browser, `(async () => {
			try {
				const res = await fetch(`+harness.JSLiteral(asset)+`);
				if (!res.ok) { return { status: res.status, error: null }; }
				const doc = new DOMParser().parseFromString(await res.text(), 'image/svg+xml');
				const failure = doc.querySelector('parsererror');
				return { status: res.status, error: failure && failure.textContent.trim() };
			} catch (e) { return { status: 0, error: String(e) }; }
		})()`)
		if err != nil {
			return err
		}
		if probe.Status != 200 {
			return fmt.Errorf("%s returned HTTP %d", asset, probe.Status)
		}
		if probe.Error != nil {
			return fmt.Errorf("%s is not well-formed XML, so a strict parser renders no icon at all:\n%s", asset, *probe.Error)
		}
		return nil
	})
}
