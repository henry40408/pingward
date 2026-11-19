package view

import (
	"math"
	"slices"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

// Bar is one heartbeat bar: height in px, CSS class, tooltip.
type Bar struct {
	Height int
	Class  string
	Title  string
}

const (
	maxHeight   = 26
	minHeight   = 5
	noneHeight  = 16
	hotFraction = 0.80
)

func isFinish(kind models.PingKind) bool {
	return kind == models.PingSuccess || kind == models.PingFail
}

func byTimeThenID(a, b models.PingSummary) int {
	if c := a.CreatedAt.Compare(b.CreatedAt); c != 0 {
		return c
	}
	return int(a.ID - b.ID)
}

// RunDurations maps a finish ping id to the seconds since the preceding start,
// in any input order.
func RunDurations(pings []models.PingSummary) map[int64]int64 {
	ordered := slices.Clone(pings)
	slices.SortFunc(ordered, byTimeThenID)
	out := map[int64]int64{}
	var startedAt *time.Time
	for _, ping := range ordered {
		if ping.Kind == models.PingStart {
			at := ping.CreatedAt
			startedAt = &at
			continue
		}
		if !isFinish(ping.Kind) || startedAt == nil {
			continue
		}
		secs := int64(ping.CreatedAt.Sub(*startedAt) / time.Second)
		startedAt = nil
		if secs >= 0 {
			out[ping.ID] = secs
		}
	}
	return out
}

// HeartbeatOptions are the inputs of Heartbeat besides the pings.
type HeartbeatOptions struct {
	MaxRuntimeSecs *int64
	Paused         bool
	Count          int
}

// Heartbeat renders the last Count runs: height by fraction of the runtime
// budget, colour by outcome.
func Heartbeat(pings []models.PingSummary, opts HeartbeatOptions) []Bar {
	if opts.Paused {
		bars := make([]Bar, opts.Count)
		for i := range bars {
			bars[i] = Bar{Height: minHeight, Class: "pausedbar", Title: "Paused"}
		}
		return bars
	}
	durations := RunDurations(pings)
	runs := []models.PingSummary{}
	for _, ping := range pings {
		if isFinish(ping.Kind) {
			runs = append(runs, ping)
		}
	}
	slices.SortFunc(runs, byTimeThenID)
	runs = runs[max(len(runs)-opts.Count, 0):]

	ceiling := runtimeCeiling(runs, durations, opts.MaxRuntimeSecs)
	bars := make([]Bar, 0, len(runs))
	for _, run := range runs {
		bars = append(bars, bar(run, durations, ceiling, opts.MaxRuntimeSecs))
	}
	return bars
}

// runtimeCeiling is the explicit max runtime, else the window max when at
// least two runs were measured.
func runtimeCeiling(runs []models.PingSummary, durations map[int64]int64, maxRuntime *int64) *int64 {
	if maxRuntime != nil && *maxRuntime > 0 {
		return maxRuntime
	}
	measured := []int64{}
	for _, run := range runs {
		if d, ok := durations[run.ID]; ok {
			measured = append(measured, d)
		}
	}
	if len(measured) < 2 {
		return nil
	}
	highest := slices.Max(measured)
	return &highest
}

func bar(run models.PingSummary, durations map[int64]int64, ceiling, maxRuntime *int64) Bar {
	duration, measured := durations[run.ID]
	failed := run.Kind == models.PingFail
	if !measured || ceiling == nil || *ceiling <= 0 {
		return unmeasuredBar(failed, measured)
	}
	fraction := math.Min(math.Max(float64(duration)/float64(*ceiling), 0), 1)
	height := min(max(int(math.Round(maxHeight*fraction)), minHeight), maxHeight)
	class := ""
	if failed {
		class = "bad"
	} else if maxRuntime != nil && *maxRuntime > 0 && float64(duration) >= hotFraction*float64(*maxRuntime) {
		class = "hot"
	}
	return Bar{Height: height, Class: class, Title: FmtSecs(duration) + " / " + FmtSecs(*ceiling)}
}

func unmeasuredBar(failed, measured bool) Bar {
	switch {
	case failed:
		return Bar{Height: maxHeight, Class: "bad", Title: "Failed"}
	case measured:
		return Bar{Height: noneHeight, Class: "none", Title: "No runtime limit set"}
	default:
		return Bar{Height: noneHeight, Class: "none", Title: "Duration unknown"}
	}
}
