package main

import "github.com/henry40408/pingward/e2e/internal/harness"

// padding goes past a shot's cut, so it ends on a card boundary.
const padding = 16.0

var (
	desktop = harness.Device{Width: 1280, Height: 900, Scale: 2}
	mobile  = harness.Device{Width: 390, Height: 844, Scale: 3, Mobile: true}
)

// region names the element a frame is measured against.
type region struct {
	kind     regionKind
	selector string // a CSS selector, card text or test id
	index    int    // the 0-based match, for byTestID
}

type regionKind int

const (
	// cardWith is `.card:has(selector)`.
	cardWith regionKind = iota
	// cardSaying is the first card containing the text.
	cardSaying
	// css is the first match.
	css
	// lastMatch is the last match.
	lastMatch
	// byTestID is the nth element with this data-testid.
	byTestID
)

// frameKind says how much of the page a shot keeps.
type frameKind int

const (
	// fullPage is the whole page.
	fullPage frameKind = iota
	// downTo is the top down to a region plus padding (0 on a list divider, or
	// the next row leaks in).
	downTo
	// band is from one region to another, for the middle of a long page.
	band
)

type frame struct {
	kind     frameKind
	from, to region
	padding  float64
}

// settleKind is a post-load step that puts the page in the state to
// photograph.
type settleKind int

const (
	// dashboardRows waits for the dashboard row at the given index.
	dashboardRows settleKind = iota
	// downCheck opens the down check from the dashboard.
	downCheck
	// downCheckExpanded also expands the newest failed run's captured output.
	downCheckExpanded
	// manageProject follows the first "Manage →" link.
	manageProject
	adminScale
	// auditExpanded expands the newest audit entry.
	auditExpanded
)

type settle struct {
	kind  settleKind
	index int
}

type shot struct {
	file   string
	scheme string
	device harness.Device
	goTo   string
	settle settle
	frame  frame
}

var shots = []shot{
	{
		file: "dashboard-dark.png", scheme: "dark", device: desktop, goTo: "/",
		settle: settle{kind: dashboardRows, index: 9}, frame: frame{kind: fullPage},
	},
	{
		file: "check-dark.png", scheme: "dark", device: desktop, goTo: "/",
		settle: settle{kind: downCheck},
		frame:  frame{kind: downTo, to: region{kind: cardSaying, selector: "Notify channels"}, padding: padding},
	},
	{
		file: "check-history-dark.png", scheme: "dark", device: desktop, goTo: "/",
		settle: settle{kind: downCheckExpanded},
		frame: frame{
			kind: band,
			from: region{kind: css, selector: "#pings-card"},
			to:   region{kind: lastMatch, selector: ".card"},
		},
	},
	{
		file: "project-dark.png", scheme: "dark", device: desktop, goTo: "/",
		settle: settle{kind: manageProject}, frame: frame{kind: fullPage},
	},
	{
		file: "admin-dark.png", scheme: "dark", device: desktop, goTo: "/admin",
		settle: settle{kind: adminScale},
		frame: frame{
			kind: downTo, padding: padding,
			to: region{kind: cardWith, selector: `[data-testid="sched-scan"]`},
		},
	},
	{
		file: "admin-audit-dark.png", scheme: "dark", device: desktop, goTo: "/admin",
		settle: settle{kind: auditExpanded},
		frame: frame{
			kind: band,
			from: region{kind: cardWith, selector: "#audit-section"},
			to:   region{kind: cardWith, selector: "#audit-section"},
		},
	},
	{
		file: "dashboard-light.png", scheme: "light", device: desktop, goTo: "/",
		settle: settle{kind: dashboardRows, index: 9}, frame: frame{kind: fullPage},
	},
	{
		file: "dashboard-mobile.png", scheme: "dark", device: mobile, goTo: "/",
		settle: settle{kind: dashboardRows, index: 1},
		frame:  frame{kind: downTo, to: region{kind: byTestID, selector: "dashboard-check-row", index: 1}},
	},
	{
		file: "check-mobile.png", scheme: "dark", device: mobile, goTo: "/",
		settle: settle{kind: downCheck},
		frame:  frame{kind: downTo, to: region{kind: cardSaying, selector: "Notify channels"}, padding: padding},
	},
}
