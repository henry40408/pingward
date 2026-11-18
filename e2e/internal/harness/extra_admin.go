package harness

// TagName is the element's upper-case tag name.
func (e *Element) TagName() (string, error) {
	return callOn[string](e, "function() { return this.tagName; }")
}

// SubmitElementWhenReady is SubmitElement for a control that may still be
// disabled: it waits until the control is clickable before it clicks.
func (b *Browser) SubmitElementWhenReady(element *Element) error {
	return b.submitWith(element.String(), element.ClickWhenReady)
}
