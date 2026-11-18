package main

import (
	"fmt"
	"math"
	"strings"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// freezeCSS removes animation and caret nondeterminism.
const freezeCSS = "*{animation:none !important;transition:none !important;caret-color:transparent !important}"

func settlePage(browser *harness.Browser, step settle) error {
	switch step.kind {
	case dashboardRows:
		return harness.Eventually("the dashboard rows", func() (bool, error) {
			rows, err := browser.TestIDs("dashboard-check-row")
			return len(rows) > step.index, err
		})
	case downCheck:
		return openDownCheck(browser)
	case downCheckExpanded:
		return expandNewestFailedRun(browser)
	case manageProject:
		return openFirstProject(browser)
	case adminScale:
		return browser.ExpectVisible("admin-scale")
	default:
		return expandNewestAuditEntry(browser)
	}
}

func openDownCheck(browser *harness.Browser) error {
	if err := browser.ExpectExactTextSomewhere("home-nas-snapshot"); err != nil {
		return err
	}
	if err := clickLink(browser, "home-nas-snapshot"); err != nil {
		return err
	}
	if err := browser.ExpectVisible("check-status"); err != nil {
		return err
	}
	return browser.ExpectVisible("ping-row")
}

func expandNewestFailedRun(browser *harness.Browser) error {
	if err := openDownCheck(browser); err != nil {
		return err
	}
	if err := browser.ClickCSS("tr.toggle"); err != nil {
		return err
	}
	return browser.ExpectVisibleCSS("tr.exp .out")
}

func openFirstProject(browser *harness.Browser) error {
	if err := clickLink(browser, "Manage →"); err != nil {
		return err
	}
	return browser.ExpectVisible("new-check-link")
}

func expandNewestAuditEntry(browser *harness.Browser) error {
	if err := browser.ExpectVisible("audit-row"); err != nil {
		return err
	}
	if err := browser.ClickCSS("#audit-section tr.toggle"); err != nil {
		return err
	}
	return browser.ExpectVisibleCSS("#audit-section tr.exp .out")
}

func clickLink(browser *harness.Browser, name string) error {
	page, err := browser.Query("body")
	if err != nil {
		return err
	}
	link, err := page.LinkNamed(name)
	if err != nil {
		return err
	}
	if link == nil {
		return fmt.Errorf("the page has no `%s` link", name)
	}
	return link.ClickWhenReady()
}

// freezePage waits for fonts and stops motion, so two runs match.
func freezePage(browser *harness.Browser) error {
	_, err := harness.Eval[any](browser, `(() => {
		const style = document.createElement('style');
		style.textContent = `+harness.JSLiteral(freezeCSS)+`;
		document.head.appendChild(style);
	})()`)
	if err != nil {
		return err
	}
	_, err = harness.EvalAsync[bool](browser, "document.fonts.ready.then(() => true)")
	return err
}

// frameClip works out the document region a shot keeps.
func frameClip(browser *harness.Browser, current shot) (harness.Clip, error) {
	width := float64(current.device.Width)
	switch current.frame.kind {
	case fullPage:
		height, err := harness.Eval[float64](browser, "document.documentElement.scrollHeight")
		return harness.Clip{Width: width, Height: height}, err
	case downTo:
		box, err := regionBox(browser, current.frame.to)
		return harness.Clip{Width: width, Height: math.Ceil(box.Y + box.Height + current.frame.padding)}, err
	default:
		return bandClip(browser, current.frame, width)
	}
}

func bandClip(browser *harness.Browser, framing frame, width float64) (harness.Clip, error) {
	head, err := regionBox(browser, framing.from)
	if err != nil {
		return harness.Clip{}, err
	}
	tail, err := regionBox(browser, framing.to)
	if err != nil {
		return harness.Clip{}, err
	}
	top := math.Floor(math.Max(head.Y-padding, 0))
	return harness.Clip{Y: top, Width: width, Height: math.Ceil(tail.Y + tail.Height + padding - top)}, nil
}

// regionBox is a region's document box: scrolling to the top first makes the
// viewport-relative rect equal it.
func regionBox(browser *harness.Browser, target region) (harness.Rect, error) {
	if _, err := harness.Eval[any](browser, "window.scrollTo(0, 0)"); err != nil {
		return harness.Rect{}, err
	}
	element, err := resolveRegion(browser, target)
	if err != nil {
		return harness.Rect{}, err
	}
	return element.Box()
}

func resolveRegion(browser *harness.Browser, target region) (*harness.Element, error) {
	switch target.kind {
	case cardWith:
		return browser.CSS(".card:has(" + target.selector + ")")
	case cardSaying:
		return cardContaining(browser, target.selector)
	case css:
		return browser.CSS(target.selector)
	case lastMatch:
		return nthMatch(browser.CSSAll(target.selector))
	default:
		elements, err := browser.TestIDs(target.selector)
		if err != nil {
			return nil, err
		}
		if len(elements) <= target.index {
			return nil, fmt.Errorf("fewer than %d elements carry `%s`", target.index+1, target.selector)
		}
		return elements[target.index], nil
	}
}

func nthMatch(elements []*harness.Element, err error) (*harness.Element, error) {
	if err != nil {
		return nil, err
	}
	if len(elements) == 0 {
		return nil, fmt.Errorf("nothing matches the selector")
	}
	return elements[len(elements)-1], nil
}

func cardContaining(browser *harness.Browser, text string) (*harness.Element, error) {
	cards, err := browser.CSSAll(".card")
	if err != nil {
		return nil, err
	}
	for _, card := range cards {
		content, err := card.TextContent()
		if err != nil {
			return nil, err
		}
		if strings.Contains(content, text) {
			return card, nil
		}
	}
	return nil, fmt.Errorf("no card says %q", text)
}
