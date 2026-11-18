package main

import (
	"strings"
	"testing"
)

func TestSquareOffFrameDropsTheRadiusOfTheFrameOnly(t *testing.T) {
	svg := "<rect data-frame=\"true\" width=\"32\" rx=\"7\" ry=\"7\"/>\n<rect x=\"4\" rx=\"2\"/>"
	got := squareOffFrame(svg)
	want := "<rect data-frame=\"true\" width=\"32\"/>\n<rect x=\"4\" rx=\"2\"/>"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestMirrorRadiusCopiesRxOnlyWhenRyIsMissing(t *testing.T) {
	got := mirrorRadius(`<rect rx="2"/><rect rx="3" ry="4"/><rect width="1"/>`)
	want := `<rect rx="2" ry="2"/><rect rx="3" ry="4"/><rect width="1"/>`
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
	if !strings.Contains(mirrorRadius(`<rect rx="2">`), `ry="2">`) {
		t.Error("a non-self-closing tag lost its end")
	}
}
