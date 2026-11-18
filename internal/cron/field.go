package cron

import (
	"fmt"
	"sort"
	"strings"
)

// ordinalSet is a sorted set of field values.
type ordinalSet []uint32

func (s ordinalSet) contains(value uint32) bool {
	index := sort.Search(len(s), func(i int) bool { return s[i] >= value })
	return index < len(s) && s[index] == value
}

// within returns the members in [low, high].
func (s ordinalSet) within(low, high uint32) []uint32 {
	var out []uint32
	for _, value := range s {
		if value >= low && value <= high {
			out = append(out, value)
		}
	}
	return out
}

type unit struct {
	name     string
	min, max uint32
	names    map[string]uint32 // nil: the field refuses names
}

var (
	secondsUnit     = &unit{name: "Seconds", min: 0, max: 59}
	minutesUnit     = &unit{name: "Minutes", min: 0, max: 59}
	hoursUnit       = &unit{name: "Hours", min: 0, max: 23}
	daysOfMonthUnit = &unit{name: "Days of Month", min: 1, max: 31}
	yearsUnit       = &unit{name: "Years", min: 1970, max: 2100}
	monthsUnit      = &unit{name: "Months", min: 1, max: 12, names: map[string]uint32{
		"jan": 1, "january": 1, "feb": 2, "february": 2, "mar": 3, "march": 3,
		"apr": 4, "april": 4, "may": 5, "jun": 6, "june": 6, "jul": 7, "july": 7,
		"aug": 8, "august": 8, "sep": 9, "september": 9, "oct": 10, "october": 10,
		"nov": 11, "november": 11, "dec": 12, "december": 12,
	}}
	// Sunday is 1 and Saturday is 7, unlike Vixie cron.
	daysOfWeekUnit = &unit{name: "Days of Week", min: 1, max: 7, names: map[string]uint32{
		"sun": 1, "sunday": 1, "mon": 2, "monday": 2, "tue": 3, "tues": 3, "tuesday": 3,
		"wed": 4, "wednesday": 4, "thu": 5, "thurs": 5, "thursday": 5,
		"fri": 6, "friday": 6, "sat": 7, "saturday": 7,
	}}
)

// message builds an error whose text is the Rust crate's verbatim, capital
// letters and trailing periods included.
func message(format string, args ...any) error {
	return &Error{text: fmt.Sprintf(format, args...)}
}

func (u *unit) supported() ordinalSet {
	set := make(ordinalSet, 0, u.max-u.min+1)
	for value := u.min; value <= u.max; value++ {
		set = append(set, value)
	}
	return set
}

func allOf(u *unit) ordinalSet { return u.supported() }

func single(u *unit, value uint32) ordinalSet {
	_ = u
	return ordinalSet{value}
}

func (u *unit) ordinalFromName(name string) (uint32, error) {
	if u.names == nil {
		return 0, message("The '%s' field does not support using names. '%s' specified.", u.name, name)
	}
	value, ok := u.names[strings.ToLower(name)]
	if ok {
		return value, nil
	}
	if u == monthsUnit {
		return 0, message("'%s' is not a valid month name.", name)
	}
	return 0, message("'%s' is not a valid day of the week.", name)
}

func (u *unit) validate(value uint32) error {
	if value < u.min {
		return message("%s must be greater than or equal to %d. ('%d' specified.)", u.name, u.min, value)
	}
	if value > u.max {
		return message("%s must be less than %d. ('%d' specified.)", u.name, u.max, value)
	}
	return nil
}

func (u *unit) ordinalsFromSpecifier(spec specifier) (ordinalSet, error) {
	switch spec.kind {
	case specAll:
		return u.supported(), nil
	case specPoint:
		return ordinalSet{spec.start}, nil
	case specRange:
		if u.validate(spec.start) != nil || u.validate(spec.end) != nil || spec.start > spec.end {
			return nil, message("Invalid range for %s: %d-%d", u.name, spec.start, spec.end)
		}
		return span(spec.start, spec.end), nil
	}
	start, err := u.ordinalFromName(spec.startName)
	if err != nil {
		return nil, err
	}
	end, err := u.ordinalFromName(spec.endName)
	if err != nil {
		return nil, err
	}
	if u.validate(start) != nil || u.validate(end) != nil || start > end {
		return nil, message("Invalid named range for %s: %s-%s", u.name, spec.startName, spec.endName)
	}
	return span(start, end), nil
}

func span(start, end uint32) ordinalSet {
	set := make(ordinalSet, 0, end-start+1)
	for value := start; value <= end; value++ {
		set = append(set, value)
	}
	return set
}

func (u *unit) ordinalsFromRoot(spec rootSpec) (ordinalSet, error) {
	switch spec.kind {
	case rootSpecifier:
		return u.ordinalsFromSpecifier(spec.spec)
	case rootNamedPoint:
		value, err := u.ordinalFromName(spec.named)
		if err != nil {
			return nil, err
		}
		return ordinalSet{value}, nil
	}
	if spec.step == 0 {
		return nil, message("range step cannot be zero")
	}
	if spec.step > u.max {
		return nil, message("%s must be between 1 and %d. ('%d' specified.)", u.name, u.max, spec.step)
	}
	var base ordinalSet
	if spec.spec.kind == specPoint {
		if err := u.validate(spec.spec.start); err != nil {
			return nil, err
		}
		base = span(spec.spec.start, u.max)
	} else {
		var err error
		if base, err = u.ordinalsFromSpecifier(spec.spec); err != nil {
			return nil, err
		}
	}
	var stepped ordinalSet
	for index := 0; index < len(base); index += int(spec.step) {
		stepped = append(stepped, base[index])
	}
	return stepped, nil
}

// resolve turns parsed specifiers into the field's value set, reporting the
// first invalid value in the crate's order.
func (u *unit) resolve(specs []rootSpec) (ordinalSet, error) {
	if len(specs) == 1 && specs[0].kind == rootSpecifier && specs[0].spec.kind == specAll {
		return u.supported(), nil
	}
	members := map[uint32]bool{}
	for _, spec := range specs {
		ordinals, err := u.ordinalsFromRoot(spec)
		if err != nil {
			return nil, err
		}
		for _, ordinal := range ordinals {
			if err := u.validate(ordinal); err != nil {
				return nil, err
			}
			members[ordinal] = true
		}
	}
	set := make(ordinalSet, 0, len(members))
	for ordinal := range members {
		set = append(set, ordinal)
	}
	sort.Slice(set, func(i, j int) bool { return set[i] < set[j] })
	return set, nil
}
