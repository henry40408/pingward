// Command icons renders the committed internal/assets/static/apple-touch-icon.png
// from internal/assets/static/favicon.svg. Rerun it after editing the SVG:
// `cd e2e && go run ./cmd/icons`.
package main

import (
	"bytes"
	"fmt"
	"image"
	"image/png"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/srwiley/oksvg"
	"github.com/srwiley/rasterx"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// iconSize is iOS's size; it downscales for smaller slots.
const iconSize = 180

// frameRadius matches the rx/ry attributes of the data-frame backing rect.
var frameRadius = regexp.MustCompile(`\s+(?:rx|ry)="[^"]*"`)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "icons:", err)
		os.Exit(1)
	}
	fmt.Printf("wrote internal/assets/static/apple-touch-icon.png (%dx%d)\n", iconSize, iconSize)
}

func run() error {
	static := filepath.Join(harness.RepoRoot(), "internal", "assets", "static")
	svg, err := os.ReadFile(filepath.Join(static, "favicon.svg"))
	if err != nil {
		return err
	}
	rendered, err := renderIcon(squareOffFrame(string(svg)))
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(static, "apple-touch-icon.png"), rendered, 0o644)
}

// squareOffFrame drops rx/ry from the data-frame backing rect: iOS applies its
// own mask, and a rounded source reads as a double-rounded edge.
func squareOffFrame(svg string) string {
	lines := strings.Split(svg, "\n")
	for index, line := range lines {
		if strings.Contains(line, "data-frame") {
			lines[index] = frameRadius.ReplaceAllString(line, "")
		}
	}
	return strings.Join(lines, "\n")
}

// tagWithRx finds a tag that sets rx; the capture group is its value.
var (
	tagWithRx = regexp.MustCompile(`<[^>]*\srx="([^"]*)"[^>]*>`)
	hasRy     = regexp.MustCompile(`\sry=`)
)

// mirrorRadius gives every rx-only tag the matching ry. The SVG spec says a
// missing ry equals rx, but oksvg reads it as 0 and draws square corners.
func mirrorRadius(svg string) string {
	return tagWithRx.ReplaceAllStringFunc(svg, func(tag string) string {
		if hasRy.MatchString(tag) {
			return tag
		}
		radius := tagWithRx.FindStringSubmatch(tag)[1]
		return strings.TrimSuffix(strings.TrimSuffix(tag, ">"), "/") + ` ry="` + radius + `"` + tagEnd(tag)
	})
}

func tagEnd(tag string) string {
	if strings.HasSuffix(tag, "/>") {
		return "/>"
	}
	return ">"
}

func renderIcon(svg string) ([]byte, error) {
	icon, err := oksvg.ReadIconStream(strings.NewReader(mirrorRadius(svg)))
	if err != nil {
		return nil, fmt.Errorf("parsing favicon.svg: %w", err)
	}
	icon.SetTarget(0, 0, iconSize, iconSize)
	canvas := image.NewRGBA(image.Rect(0, 0, iconSize, iconSize))
	icon.Draw(rasterx.NewDasher(iconSize, iconSize, rasterx.NewScannerGV(iconSize, iconSize, canvas, canvas.Bounds())), 1)
	var encoded bytes.Buffer
	if err := png.Encode(&encoded, canvas); err != nil {
		return nil, err
	}
	return encoded.Bytes(), nil
}
