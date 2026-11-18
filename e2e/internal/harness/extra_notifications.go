package harness

// Checked reports whether a checkbox is ticked right now.
func (e *Element) Checked() (bool, error) {
	return callOn[bool](e, "function() { return this.checked; }")
}
