package e2e

import (
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// healthTableIDs are the admin health tables; they render only once there is
// failing data.
var healthTableIDs = []string{"health-down", "health-channels", "health-recent"}

// healthTimeout is how long to keep reloading until the failed delivery is
// recorded.
const healthTimeout = 30 * time.Second

// phoneHeight is the viewport height of every phone-width step.
const phoneHeight = 667

// probe is the object a layout probe script returns.
type probe map[string]any

func (p probe) number(field string) float64 {
	value, _ := p[field].(float64)
	return value
}

// count is for integer counts, so comparisons avoid float equality.
func (p probe) count(field string) int { return int(p.number(field)) }

func (p probe) text(field string) string {
	value, _ := p[field].(string)
	return value
}

func (p probe) flag(field string) bool {
	value, _ := p[field].(bool)
	return value
}

// measure evaluates a probe body, a function body with `return`, in the page.
func (w *world) measure(body string) (probe, error) {
	return harness.Eval[probe](w.browser, "(() => {\n"+body+"\n})()")
}

// registerMobileLayoutSteps covers the phone-width layout invariants, measured
// rather than read from markup: the behaviour lives entirely in CSS.
func registerMobileLayoutSteps(sc *godog.ScenarioContext, w *world) {
	sc.Step(`^I view the site at (\d+)px wide$`, func(width int) error {
		return w.browser.SetViewport(int64(width), phoneHeight)
	})
	sc.Step(`^the page has no horizontal scrollbar$`, w.pageHasNoHorizontalScrollbar)
	sc.Step(`^only the users table scrolls sideways, not the card around it$`, w.usersTableContained)
	sc.Step(`^Environment rows do not wrap$`, w.environmentRowsDoNotWrap)
	sc.Step(`^the admin health tables are shown$`, w.healthTablesShown)
	sc.Step(`^each admin health table scrolls inside its card, not the card around it$`, w.healthTablesContained)
	sc.Step(`^the heartbeat legend sits on its own row below the edge captions$`, w.heartbeatLegendBelowCaptions)
	sc.Step(`^I open the project from the breadcrumb$`, w.openProjectFromBreadcrumb)
	sc.Step(`^the check row's status dot sits next to the name$`, w.statusDotNextToName)
	sc.Step(`^the group header's count and manage link each stay on one line$`, w.groupHeaderLabelsUnwrapped)
	sc.Step(`^the check row's name stays on one line beside the `+quoted+` chip$`, func(chip string) error {
		return w.nameStaysBesideChip(unquote(chip))
	})
	sc.Step(`^the check row is a single line$`, w.checkRowSingleLine)
}

func (w *world) pageHasNoHorizontalScrollbar() error {
	measured, err := w.measure(`return {
		scrollW: document.documentElement.scrollWidth,
		clientW: document.documentElement.clientWidth,
	};`)
	if err != nil {
		return err
	}
	scroll, client := measured.number("scrollW"), measured.number("clientW")
	if scroll > client {
		return fmt.Errorf("page scrolls horizontally: scrollWidth %vpx > viewport %vpx", scroll, client)
	}
	return nil
}

// `.cb` has its own `overflow-x`, so an unwrapped table scrolls the whole
// body, Add-user form included; `.tscroll` confines it to the table.
func (w *world) usersTableContained() error {
	measured, err := w.measure(`const cb = document.querySelector('[data-testid="user-submit"]').closest(".cb");
		const table = cb.querySelector("table");
		return {
			bodyOverflow: cb.scrollWidth - cb.clientWidth,
			wrapper: table.parentElement.className,
		};`)
	if err != nil {
		return err
	}
	// Symptom first; the wrapper check is a diagnostic.
	if overflow := measured.number("bodyOverflow"); overflow > 0 {
		return fmt.Errorf("the card body itself scrolls by %vpx, so the Add-user form moves with the table", overflow)
	}
	if wrapper := measured.text("wrapper"); !strings.Contains(wrapper, "tscroll") {
		return fmt.Errorf("the users table is not wrapped in .tscroll (wrapper class: %q)", wrapper)
	}
	return nil
}

// The table already scrolls inside `.tscroll`, so wrapping only costs height.
// The one-line check is exact; the height bound is font-dependent (58px macOS,
// 78px Linux CI), so it is set well below the wrapped case.
func (w *world) environmentRowsDoNotWrap() error {
	measured, err := w.measure(`const rows = [...document.querySelectorAll('tr[data-testid^="env-row-"]')];
		const tallest = rows.reduce((worst, r) => {
			const h = Math.round(r.getBoundingClientRect().height);
			return h > worst.h ? { h, id: r.dataset.testid } : worst;
		}, { h: 0, id: "none" });
		const code = document
			.querySelector('tr[data-testid="env-row-DATABASE_URL"]')
			.querySelector("code");
		return {
			tallestHeight: tallest.h,
			tallestId: tallest.id,
			valueLines: code.getClientRects().length,
		};`)
	if err != nil {
		return err
	}
	if lines := measured.count("valueLines"); lines != 1 {
		return fmt.Errorf("the DATABASE_URL value spans %d lines — it is wrapping", lines)
	}
	if height := measured.number("tallestHeight"); height > 120 {
		return fmt.Errorf("%s is %vpx tall — the description column is being squeezed", measured.text("tallestId"), height)
	}
	return nil
}

// Delivery is a background goroutine, so the failure can be recorded after
// this page load; poll so the next step never fails on "not yet".
func (w *world) healthTablesShown() error {
	return harness.EventuallyWithin(healthTimeout, "the admin health tables", func() (bool, error) {
		if err := w.browser.Reload(); err != nil {
			return false, err
		}
		for _, id := range healthTableIDs {
			visible, err := w.browser.IsVisible(id)
			if err != nil || !visible {
				return false, err
			}
		}
		return true, nil
	})
}

// Per table: wrapped in `.tscroll`, actually overflowing (or the test is
// vacuous), card body not scrolling. Wrapper checks go first because the two
// Notification health tables share one `.cb`, so the body symptom cannot name
// the culprit.
func (w *world) healthTablesContained() error {
	results, err := harness.Eval[[]probe](w.browser, `((ids) => ids.map((id) => {
		const table = document.querySelector(`+"`[data-testid=\"${id}\"]`"+`);
		const wrapper = table.parentElement;
		const cb = table.closest(".cb");
		return {
			id,
			wrapperClass: wrapper.className,
			bodyOverflow: cb.scrollWidth - cb.clientWidth,
			wrapperOverflow: wrapper.scrollWidth - wrapper.clientWidth,
		};
	}))(`+harness.JSLiteral(healthTableIDs)+`)`)
	if err != nil {
		return err
	}
	if len(results) != len(healthTableIDs) {
		return fmt.Errorf("the containment probe measured %d of %d tables", len(results), len(healthTableIDs))
	}
	for _, result := range results {
		id, class := result.text("id"), result.text("wrapperClass")
		if !strings.Contains(class, "tscroll") {
			return fmt.Errorf("%s: table is not wrapped in .tscroll (wrapper class: %q)", id, class)
		}
		if overflow := result.number("wrapperOverflow"); overflow <= 0 {
			return fmt.Errorf("%s: the table's own wrapper does not overflow (%vpx) — "+
				"the seeded content is not wide enough to prove containment", id, overflow)
		}
	}
	for _, result := range results {
		if overflow := result.number("bodyOverflow"); overflow > 0 {
			return fmt.Errorf("%s: the card body itself scrolls by %vpx, dragging sibling content with it",
				result.text("id"), overflow)
		}
	}
	return nil
}

// Line boxes are counted via a Range: a flex item's own rects report one box
// however its text wraps. The legend starting below the captions is what proves
// the full-width row; without it its top is level with theirs.
func (w *world) heartbeatLegendBelowCaptions() error {
	measured, err := w.measure(`const cap = document.querySelector(".beatcap");
		const lineBoxes = (el) => {
			const range = document.createRange();
			range.selectNodeContents(el);
			return range.getClientRects().length;
		};
		const key = cap.querySelector(".key");
		const edges = [...cap.children].filter((el) => el !== key);
		return {
			edges: edges.map((el) => ({ text: el.textContent, lines: lineBoxes(el) })),
			edgeBottom: Math.round(Math.max(...edges.map((el) =>
				el.getBoundingClientRect().bottom))),
			keyTop: Math.round(key.getBoundingClientRect().top),
		};`)
	if err != nil {
		return err
	}
	edges, _ := measured["edges"].([]any)
	for _, raw := range edges {
		edge, _ := raw.(map[string]any)
		caption, _ := edge["text"].(string)
		lines, _ := edge["lines"].(float64)
		if lines != 1 {
			return fmt.Errorf("%q spans %v lines — it wrapped", caption, lines)
		}
	}
	keyTop, edgeBottom := measured.number("keyTop"), measured.number("edgeBottom")
	if keyTop < edgeBottom {
		return fmt.Errorf("the legend starts at y=%v, above the edge captions' bottom edge (%v) — it is still sharing their row",
			keyTop, edgeBottom)
	}
	return nil
}

func (w *world) openProjectFromBreadcrumb() error {
	crumbs, err := w.browser.CSSAll(".crumb a")
	if err != nil {
		return err
	}
	if len(crumbs) < 2 {
		return errors.New("the breadcrumb has fewer than two links")
	}
	if err := w.browser.SubmitElement(crumbs[1]); err != nil {
		return err
	}
	return w.expectPathMatching(`/projects/\d+$`)
}

// `.check`'s own gap is 16px; a layout that fits only the dashboard's children
// strands the dot far from the name on the project page.
func (w *world) statusDotNextToName() error {
	gap, err := harness.Eval[float64](w.browser, `(() => {
		const row = document.querySelector(".check");
		const dot = row.querySelector(".status-dot").getBoundingClientRect();
		const meta = row.querySelector(".cmeta").getBoundingClientRect();
		return Math.round(meta.left - dot.right);
	})()`)
	if err != nil {
		return err
	}
	if gap > 20 {
		return fmt.Errorf(".status-dot sits %vpx from .cmeta", gap)
	}
	return nil
}

// Range-based line-box counting, as above. The description must be truncating,
// or a header that simply fits passes vacuously.
func (w *world) groupHeaderLabelsUnwrapped() error {
	measured, err := w.measure(`const gh = document.querySelector(".group > .gh");
		const lineBoxes = (sel) => {
			const range = document.createRange();
			range.selectNodeContents(gh.querySelector(sel));
			return range.getClientRects().length;
		};
		const truncated = (sel) => {
			const e = gh.querySelector(sel);
			return e.scrollWidth > e.clientWidth;
		};
		return {
			count: lineBoxes(".count"),
			link: lineBoxes("a"),
			descTruncated: truncated(".gdesc"),
			nameTruncated: truncated("h2"),
		};`)
	if err != nil {
		return err
	}
	if !measured.flag("descTruncated") {
		return errors.New("the description is not being truncated, so the header is not under width pressure " +
			"and the checks below prove nothing")
	}
	if lines := measured.count("count"); lines != 1 {
		return fmt.Errorf("\"N checks\" spans %d lines — it wrapped", lines)
	}
	if lines := measured.count("link"); lines != 1 {
		return fmt.Errorf("\"Manage →\" spans %d lines — it wrapped", lines)
	}
	// Pinning the labels must not be paid for out of the project name.
	if measured.flag("nameTruncated") {
		return errors.New("the project name is truncated — the description should have absorbed the whole squeeze")
	}
	return nil
}

// Lines are counted by distinct line tops: `.nm` holds a link, and a Range over
// an element yields a rect for its box plus one for its text at the same y. The
// chip assertions are the non-vacuity guard.
func (w *world) nameStaysBesideChip(chip string) error {
	measured, err := w.measure(`const row = document.querySelector(".check");
		const nm = row.querySelector(".cmeta .nm");
		const range = document.createRange();
		range.selectNodeContents(nm);
		const badge = row.querySelector('[data-testid="check-no-channel"]');
		const box = badge && badge.getBoundingClientRect();
		const rects = [...range.getClientRects()];
		return {
			lines: new Set(rects.map((r) => Math.round(r.top))).size,
			rects: rects.length,
			name: nm.textContent,
			chip: badge ? badge.textContent : "",
			chipWidth: box ? Math.round(box.width) : 0,
			chipTop: box ? Math.round(box.top) : 0,
			metaBottom: Math.round(
				row.querySelector(".cmeta").getBoundingClientRect().bottom),
		};`)
	if err != nil {
		return err
	}
	if rendered := measured.text("chip"); rendered != chip {
		return errors.New("the row renders no chip, so the line count below is vacuous")
	}
	if measured.number("chipWidth") <= 0 {
		return fmt.Errorf("the %q chip is in the DOM but has no box — it is still display:none at this width", chip)
	}
	if lines := measured.count("lines"); lines != 1 {
		return fmt.Errorf("the check name %q spans %d lines (%d client rects) — the chip squeezed .cmeta until it wrapped",
			measured.text("name"), lines, measured.count("rects"))
	}
	// The chip must sit on its own row below; sharing the name's row only works
	// for short names.
	chipTop, metaBottom := measured.number("chipTop"), measured.number("metaBottom")
	if chipTop < metaBottom {
		return fmt.Errorf("the chip starts at y=%v, above the name block's bottom edge (%v) — it is still sharing the row",
			chipTop, metaBottom)
	}
	return nil
}

// On one line the badge's and dot's vertical centres coincide.
func (w *world) checkRowSingleLine() error {
	drop, err := harness.Eval[float64](w.browser, `(() => {
		const row = document.querySelector(".check");
		const centre = (sel) => {
			const r = row.querySelector(sel).getBoundingClientRect();
			return r.top + r.height / 2;
		};
		return Math.round(Math.abs(centre(".badge") - centre(".status-dot")));
	})()`)
	if err != nil {
		return err
	}
	if drop > 2 {
		return fmt.Errorf(".badge sits %vpx below the .status-dot — it wrapped", drop)
	}
	return nil
}
