// Package cron ports the Rust `cron` 0.17.0 crate: a 6-field (sec min hour dom
// mon dow, optional year) schedule parser and next-trigger search, including
// the crate's error texts, which come from its winnow parser.
package cron

import (
	"fmt"
	"strconv"
	"strings"
)

// Error is the text the Rust crate's Display produces for a bad expression.
type Error struct{ text string }

func (e *Error) Error() string { return e.text }

// failure is a winnow backtrack. cause is non-empty only for errors raised by
// try_map; the parser position at the time of failure decides the caret.
type failure struct{ cause string }

type parser struct {
	input string
	pos   int
}

type specifierKind int

const (
	specAll specifierKind = iota
	specPoint
	specRange
	specNamedRange
)

type specifier struct {
	kind       specifierKind
	start, end uint32
	startName  string
	endName    string
}

type rootKind int

const (
	rootSpecifier rootKind = iota
	rootPeriod
	rootNamedPoint
)

type rootSpec struct {
	kind  rootKind
	spec  specifier
	step  uint32
	named string
}

func (p *parser) rest() string { return p.input[p.pos:] }

func (p *parser) takeWhile(accept func(byte) bool) string {
	start := p.pos
	for p.pos < len(p.input) && accept(p.input[p.pos]) {
		p.pos++
	}
	return p.input[start:p.pos]
}

func (p *parser) multispace0() {
	p.takeWhile(func(b byte) bool { return b == ' ' || b == '\t' || b == '\r' || b == '\n' })
}

func isDigit(b byte) bool { return b >= '0' && b <= '9' }

func isAlpha(b byte) bool { return b >= 'a' && b <= 'z' || b >= 'A' && b <= 'Z' }

func (p *parser) takeWhile1(accept func(byte) bool) (string, *failure) {
	run := p.takeWhile(accept)
	if run == "" {
		return "", &failure{}
	}
	return run, nil
}

func (p *parser) literal(text string) *failure {
	if !strings.HasPrefix(p.rest(), text) {
		return &failure{}
	}
	p.pos += len(text)
	return nil
}

// alt mirrors winnow: the input is reset before every branch but not after the
// last failure, so the caret lands where the last branch gave up.
func alt[T any](p *parser, branches ...func() (T, *failure)) (T, *failure) {
	start := p.pos
	var lastFailure *failure
	var zero T
	for _, branch := range branches {
		p.pos = start
		value, fail := branch()
		if fail == nil {
			return value, nil
		}
		lastFailure = fail
	}
	return zero, lastFailure
}

func (p *parser) ordinal() (uint32, *failure) {
	start := p.pos
	p.multispace0()
	digits, fail := p.takeWhile1(isDigit)
	if fail != nil {
		return 0, fail
	}
	p.multispace0()
	value, err := strconv.ParseUint(digits, 10, 32)
	if err != nil {
		p.pos = start
		return 0, &failure{cause: "number too large to fit in target type"}
	}
	return uint32(value), nil
}

func (p *parser) name() (string, *failure) {
	p.multispace0()
	letters, fail := p.takeWhile1(isAlpha)
	if fail != nil {
		return "", fail
	}
	p.multispace0()
	return letters, nil
}

func (p *parser) point() (specifier, *failure) {
	value, fail := p.ordinal()
	if fail != nil {
		return specifier{}, fail
	}
	return specifier{kind: specPoint, start: value}, nil
}

func (p *parser) rangeSpec() (specifier, *failure) {
	start, fail := p.ordinal()
	if fail != nil {
		return specifier{}, fail
	}
	if fail := p.literal("-"); fail != nil {
		return specifier{}, fail
	}
	end, fail := p.ordinal()
	if fail != nil {
		return specifier{}, fail
	}
	return specifier{kind: specRange, start: start, end: end}, nil
}

func (p *parser) namedRange() (specifier, *failure) {
	startName, fail := p.name()
	if fail != nil {
		return specifier{}, fail
	}
	if fail := p.literal("-"); fail != nil {
		return specifier{}, fail
	}
	endName, fail := p.name()
	if fail != nil {
		return specifier{}, fail
	}
	return specifier{kind: specNamedRange, startName: startName, endName: endName}, nil
}

func (p *parser) star() (specifier, *failure) {
	return specifier{kind: specAll}, p.literal("*")
}

func (p *parser) question() (specifier, *failure) {
	return specifier{kind: specAll}, p.literal("?")
}

func (p *parser) specifier() (specifier, *failure) {
	return alt(p, p.star, p.rangeSpec, p.point, p.namedRange)
}

func (p *parser) specifierWithAny() (specifier, *failure) {
	return alt(p, p.question, p.specifier)
}

func (p *parser) period(withAny bool) (rootSpec, *failure) {
	base := p.specifier
	if withAny {
		base = p.specifierWithAny
	}
	start, fail := base()
	if fail != nil {
		return rootSpec{}, fail
	}
	if fail := p.literal("/"); fail != nil {
		return rootSpec{}, fail
	}
	step, fail := p.ordinal()
	if fail != nil {
		return rootSpec{}, fail
	}
	return rootSpec{kind: rootPeriod, spec: start, step: step}, nil
}

func (p *parser) namedPoint() (rootSpec, *failure) {
	named, fail := p.name()
	if fail != nil {
		return rootSpec{}, fail
	}
	return rootSpec{kind: rootNamedPoint, named: named}, nil
}

func (p *parser) rootSpecifier(withAny bool) (rootSpec, *failure) {
	base := p.specifier
	if withAny {
		base = p.specifierWithAny
	}
	return alt(p,
		func() (rootSpec, *failure) { return p.period(withAny) },
		func() (rootSpec, *failure) {
			spec, fail := base()
			return rootSpec{kind: rootSpecifier, spec: spec}, fail
		},
		p.namedPoint,
	)
}

// separated1 is winnow's separated(1.., root, ","): a separator that is not
// followed by an element is given back.
func (p *parser) separated1(withAny bool) ([]rootSpec, *failure) {
	first, fail := p.rootSpecifier(withAny)
	if fail != nil {
		return nil, fail
	}
	items := []rootSpec{first}
	for {
		beforeSeparator := p.pos
		if fail := p.literal(","); fail != nil {
			p.pos = beforeSeparator
			return items, nil
		}
		item, fail := p.rootSpecifier(withAny)
		if fail != nil {
			p.pos = beforeSeparator
			return items, nil
		}
		items = append(items, item)
	}
}

func (p *parser) field(withAny bool) ([]rootSpec, *failure) {
	p.multispace0()
	items, fail := alt(p,
		func() ([]rootSpec, *failure) { return p.separated1(withAny) },
		func() ([]rootSpec, *failure) {
			item, fail := p.rootSpecifier(withAny)
			return []rootSpec{item}, fail
		},
	)
	if fail != nil {
		return nil, fail
	}
	p.multispace0()
	return items, nil
}

// fieldOf parses one field and resolves it; a resolve error rewinds to the
// field start like winnow's try_map.
func (p *parser) fieldOf(unit *unit, withAny bool) (ordinalSet, *failure) {
	start := p.pos
	specs, fail := p.field(withAny)
	if fail != nil {
		return nil, fail
	}
	set, err := unit.resolve(specs)
	if err != nil {
		p.pos = start
		return nil, &failure{cause: err.Error()}
	}
	return set, nil
}

var shorthands = []struct {
	keyword string
	build   func() *Schedule
}{
	{"@yearly", func() *Schedule {
		return newSchedule(single(secondsUnit, 0), single(minutesUnit, 0), single(hoursUnit, 0),
			single(daysOfMonthUnit, 1), single(monthsUnit, 1), allOf(daysOfWeekUnit), allOf(yearsUnit))
	}},
	{"@monthly", func() *Schedule {
		return newSchedule(single(secondsUnit, 0), single(minutesUnit, 0), single(hoursUnit, 0),
			single(daysOfMonthUnit, 1), allOf(monthsUnit), allOf(daysOfWeekUnit), allOf(yearsUnit))
	}},
	{"@weekly", func() *Schedule {
		return newSchedule(single(secondsUnit, 0), single(minutesUnit, 0), single(hoursUnit, 0),
			allOf(daysOfMonthUnit), allOf(monthsUnit), single(daysOfWeekUnit, 1), allOf(yearsUnit))
	}},
	{"@daily", func() *Schedule {
		return newSchedule(single(secondsUnit, 0), single(minutesUnit, 0), single(hoursUnit, 0),
			allOf(daysOfMonthUnit), allOf(monthsUnit), allOf(daysOfWeekUnit), allOf(yearsUnit))
	}},
	{"@hourly", func() *Schedule {
		return newSchedule(single(secondsUnit, 0), single(minutesUnit, 0), allOf(hoursUnit),
			allOf(daysOfMonthUnit), allOf(monthsUnit), allOf(daysOfWeekUnit), allOf(yearsUnit))
	}},
}

func (p *parser) shorthand() (*Schedule, *failure) {
	p.multispace0()
	var branches []func() (*Schedule, *failure)
	for _, entry := range shorthands {
		branches = append(branches, func() (*Schedule, *failure) {
			if fail := p.literal(entry.keyword); fail != nil {
				return nil, fail
			}
			return entry.build(), nil
		})
	}
	schedule, fail := alt(p, branches...)
	if fail != nil {
		return nil, fail
	}
	p.multispace0()
	return schedule, nil
}

func (p *parser) eof() *failure {
	if p.pos != len(p.input) {
		return &failure{}
	}
	return nil
}

func (p *parser) longhand() (*Schedule, *failure) {
	seconds, fail := p.fieldOf(secondsUnit, false)
	if fail != nil {
		return nil, fail
	}
	minutes, fail := p.fieldOf(minutesUnit, false)
	if fail != nil {
		return nil, fail
	}
	hours, fail := p.fieldOf(hoursUnit, false)
	if fail != nil {
		return nil, fail
	}
	daysOfMonth, fail := p.fieldOf(daysOfMonthUnit, true)
	if fail != nil {
		return nil, fail
	}
	months, fail := p.fieldOf(monthsUnit, false)
	if fail != nil {
		return nil, fail
	}
	daysOfWeek, fail := p.fieldOf(daysOfWeekUnit, true)
	if fail != nil {
		return nil, fail
	}
	// opt(): a bad or missing year field is dropped and the trailing eof
	// check then reports it at the field start.
	yearStart := p.pos
	years, fail := p.fieldOf(yearsUnit, false)
	if fail != nil {
		p.pos = yearStart
		years = allOf(yearsUnit)
	}
	if fail := p.eof(); fail != nil {
		return nil, fail
	}
	return newSchedule(seconds, minutes, hours, daysOfMonth, months, daysOfWeek, years), nil
}

// Parse parses a cron expression exactly like Rust's Schedule::from_str.
func Parse(expression string) (*Schedule, error) {
	p := &parser{input: expression}
	schedule, fail := alt(p, p.shorthand, p.longhand)
	if fail == nil {
		fail = p.eof()
	}
	if fail != nil {
		return nil, &Error{text: renderFailure(expression, p.pos, fail.cause)}
	}
	return schedule, nil
}

// renderFailure reproduces winnow's ParseError Display.
func renderFailure(input string, offset int, cause string) string {
	var out strings.Builder
	if !strings.Contains(input, "\n") {
		out.WriteString(input + "\n" + strings.Repeat(" ", offset) + "^\n" + cause)
		return out.String()
	}
	line, column := translatePosition(input, offset)
	lineNumber := line + 1
	gutter := strings.Repeat(" ", len(strconv.Itoa(lineNumber)))
	content := strings.Split(input, "\n")[line]
	fmt.Fprintf(&out, "parse error at line %d, column %d\n", lineNumber, column+1)
	out.WriteString(gutter + " |\n")
	fmt.Fprintf(&out, "%d | %s\n", lineNumber, content)
	out.WriteString(gutter + " | " + strings.Repeat(" ", column) + "^\n" + cause)
	return out.String()
}

// translatePosition mirrors winnow's translate_position (byte offset to
// zero-based line and column).
func translatePosition(input string, index int) (line, column int) {
	if input == "" {
		return 0, index
	}
	safeIndex := min(index, len(input)-1)
	columnOffset := index - safeIndex
	lineStart := strings.LastIndex(input[:safeIndex], "\n") + 1
	line = strings.Count(input[:lineStart], "\n")
	column = len([]rune(input[lineStart:safeIndex+1])) - 1
	return line, column + columnOffset
}
