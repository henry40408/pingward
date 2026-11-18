package e2e

import (
	"fmt"
	"math"
	"regexp"
	"strconv"
	"strings"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// minimumContrast is the WCAG AA ratio for normal text.
const minimumContrast = 4.5

// registerThemeSteps covers the three-state theme control and the palette it
// resolves.
func registerThemeSteps(sc *godog.ScenarioContext, w *world) {
	// Always `light` or `dark`, never `system`.
	sc.Step(`^the resolved theme is `+quoted+`$`, func(theme string) error {
		return w.browser.ExpectAttrIs("html", "data-theme", unquote(theme))
	})
	sc.Step(`^the stored theme preference is `+quoted+`$`, func(preference string) error {
		return harness.EventuallyEq("the stored theme preference", unquote(preference), func() (string, error) {
			// getItem is null when unset, which reads as the empty string.
			stored, err := harness.Eval[*string](w.browser, `localStorage.getItem('pw-theme')`)
			if err != nil || stored == nil {
				return "", err
			}
			return *stored, nil
		})
	})
	sc.Step(`^I click the theme toggle$`, func() error { return w.browser.ClickCSS("#pw-theme-toggle") })
	// Reloads, so `theme-init.js` re-resolves it before first paint.
	sc.Step(`^I set the theme preference to `+quoted+`$`, func(preference string) error {
		script := "localStorage.setItem('pw-theme', " + harness.JSLiteral(unquote(preference)) + "); true"
		if _, err := harness.Eval[bool](w.browser, script); err != nil {
			return err
		}
		return w.browser.Reload()
	})
	// `app.js`'s `matchMedia` listener re-resolves under `system`.
	sc.Step(`^the OS prefers dark$`, func() error { return w.browser.EmulateColorScheme("dark") })
	sc.Step(`^the OS prefers light$`, func() error { return w.browser.EmulateColorScheme("light") })
	sc.Step(`^I hover the dashboard's primary action$`, func() error {
		if err := w.goTo("/"); err != nil {
			return err
		}
		button, err := w.browser.CSS(".btn-primary")
		if err != nil {
			return err
		}
		return button.Hover()
	})
	sc.Step(`^its label contrasts with its background$`, w.labelContrasts)
}

// colourChannels reads a computed colour as 0-255 channels. Chromium reports
// `color(srgb …)` as 0-1 floats from `color-mix()`; read as 0-255 they look
// near-black and fake a pass.
func colourChannels(colour string) ([3]float64, error) {
	numbers := regexp.MustCompile(`[\d.]+`).FindAllString(colour, -1)
	var channels [3]float64
	if len(numbers) < 3 {
		return channels, fmt.Errorf("cannot read the colour %q", colour)
	}
	for i := range channels {
		value, err := strconv.ParseFloat(numbers[i], 64)
		if err != nil {
			return channels, fmt.Errorf("cannot read the colour %q: %w", colour, err)
		}
		if strings.HasPrefix(colour, "color(") {
			value *= 255
		}
		channels[i] = value
	}
	return channels, nil
}

func relativeLuminance(channels [3]float64) float64 {
	weights := [3]float64{0.2126, 0.7152, 0.0722}
	var luminance float64
	for i, channel := range channels {
		v := channel / 255
		if v <= 0.03928 {
			v /= 12.92
		} else {
			v = math.Pow((v+0.055)/1.055, 2.4)
		}
		luminance += v * weights[i]
	}
	return luminance
}

// WCAG contrast of the computed colours (`filter` is not folded in).
func (w *world) labelContrasts() error {
	button, err := w.browser.CSS(".btn-primary")
	if err != nil {
		return err
	}
	var luminances [2]float64
	for i, property := range []string{"color", "background-color"} {
		value, err := button.ComputedStyle(property)
		if err != nil {
			return err
		}
		channels, err := colourChannels(value)
		if err != nil {
			return err
		}
		luminances[i] = relativeLuminance(channels)
	}
	ratio := (math.Max(luminances[0], luminances[1]) + 0.05) / (math.Min(luminances[0], luminances[1]) + 0.05)
	if ratio < minimumContrast {
		return fmt.Errorf("the contrast ratio is %.2f, below %v", ratio, minimumContrast)
	}
	return nil
}
