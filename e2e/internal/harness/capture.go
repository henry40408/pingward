package harness

import (
	"github.com/chromedp/cdproto/emulation"
	"github.com/chromedp/cdproto/page"
	"github.com/chromedp/chromedp"
)

// Device is a screen to emulate: the viewport in CSS pixels, its pixel
// density, and whether it is a phone.
type Device struct {
	Width, Height int64
	Scale         float64
	Mobile        bool
}

// maxTouchPoints is what a phone reports.
const maxTouchPoints = 5

// EmulateDevice applies the screen size, density and touch support.
func (b *Browser) EmulateDevice(device Device) error {
	err := send(b, emulation.SetDeviceMetricsOverride, emulation.SetDeviceMetricsOverrideParams{
		Width:             device.Width,
		Height:            device.Height,
		DeviceScaleFactor: device.Scale,
		Mobile:            device.Mobile,
	})
	if err != nil {
		return err
	}
	return send(b, emulation.SetTouchEmulationEnabled, emulation.SetTouchEmulationEnabledParams{
		Enabled:        device.Mobile,
		MaxTouchPoints: maxTouchPoints,
	})
}

// EmulateLocale pins the time zone and locale, so dates render the same on
// every machine.
func (b *Browser) EmulateLocale(timezone, locale string) error {
	err := send(b, emulation.SetTimezoneOverride, emulation.SetTimezoneOverrideParams{TimezoneID: timezone})
	if err != nil {
		return err
	}
	return send(b, emulation.SetLocaleOverride, emulation.SetLocaleOverrideParams{Locale: locale})
}

// EmulateStillMedia sets the colour scheme and asks for reduced motion. Both
// features go in one call: a second call would replace the first's list.
func (b *Browser) EmulateStillMedia(scheme string) error {
	return send(b, emulation.SetEmulatedMedia, emulation.SetEmulatedMediaParams{
		Media: "screen",
		Features: []*emulation.MediaFeature{
			{Name: "prefers-color-scheme", Value: scheme},
			{Name: "prefers-reduced-motion", Value: "reduce"},
		},
	})
}

// Clip is a region of the document, in CSS pixels.
type Clip struct{ X, Y, Width, Height float64 }

// CaptureClip renders a PNG of the clip. It may be taller than the window.
// The clip's own scale stays 1, since the device scale factor already applies.
func (b *Browser) CaptureClip(clip Clip) ([]byte, error) {
	beyondViewport := true
	result, err := chromedp.Call(b.ctx, page.CaptureScreenshot, page.CaptureScreenshotParams{
		Format:                page.CaptureScreenshotFormatPng,
		CaptureBeyondViewport: &beyondViewport,
		Clip:                  &page.Viewport{X: clip.X, Y: clip.Y, Width: clip.Width, Height: clip.Height, Scale: 1},
	})
	if err != nil {
		return nil, err
	}
	return result.Data, nil
}
