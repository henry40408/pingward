package cron

import (
	"time"
)

// Schedule is a parsed expression.
type Schedule struct {
	seconds, minutes, hours, daysOfMonth, months, daysOfWeek, years ordinalSet
}

func newSchedule(seconds, minutes, hours, daysOfMonth, months, daysOfWeek, years ordinalSet) *Schedule {
	return &Schedule{seconds, minutes, hours, daysOfMonth, months, daysOfWeek, years}
}

// searchState is the crate's NextAfterQuery: the first pass through each unit
// starts at the anchor's own value, later passes at the unit minimum.
type searchState struct {
	initial                                             time.Time
	firstMonth, firstDay, firstHour, firstMin, firstSec bool
}

func newSearchState(initial time.Time) *searchState {
	return &searchState{initial, true, true, true, true, true}
}

func (q *searchState) monthLowerBound() uint32 {
	if q.firstMonth {
		q.firstMonth = false
		return uint32(q.initial.Month())
	}
	return monthsUnit.min
}

func (q *searchState) dayLowerBound() uint32 {
	if q.firstDay {
		q.firstDay = false
		return uint32(q.initial.Day())
	}
	return daysOfMonthUnit.min
}

func (q *searchState) hourLowerBound() uint32 {
	if q.firstHour {
		q.firstHour = false
		return uint32(q.initial.Hour())
	}
	return hoursUnit.min
}

func (q *searchState) minuteLowerBound() uint32 {
	if q.firstMin {
		q.firstMin = false
		return uint32(q.initial.Minute())
	}
	return minutesUnit.min
}

func (q *searchState) secondLowerBound() uint32 {
	if q.firstSec {
		q.firstSec = false
		return uint32(q.initial.Second())
	}
	return secondsUnit.min
}

func (q *searchState) resetSecond() { q.firstSec = false }
func (q *searchState) resetMinute() { q.firstMin = false; q.resetSecond() }
func (q *searchState) resetHour()   { q.firstHour = false; q.resetMinute() }
func (q *searchState) resetDay()    { q.firstDay = false; q.resetHour() }
func (q *searchState) resetMonth()  { q.firstMonth = false; q.resetDay() }

func isLeapYear(year uint32) bool {
	return year%4 == 0 && (year%100 != 0 || year%400 == 0)
}

func daysInMonth(month, year uint32) uint32 {
	switch {
	case month == 9 || month == 4 || month == 6 || month == 11:
		return 30
	case month == 2 && isLeapYear(year):
		return 29
	case month == 2:
		return 28
	}
	return 31
}

// weekdayFromSunday numbers Sunday 1 to Saturday 7, like chrono's
// number_from_sunday.
func weekdayFromSunday(year, month, day uint32) uint32 {
	date := time.Date(int(year), time.Month(month), int(day), 0, 0, 0, 0, time.UTC)
	return uint32(date.Weekday()) + 1
}

func (s *Schedule) daysOfWeekIsAll() bool {
	return len(s.daysOfWeek) == int(daysOfWeekUnit.max-daysOfWeekUnit.min+1)
}

// lastTransitionYear is the last year chrono-tz 0.10 carries zone rules for;
// from 2038 on its zones keep the offset they had at the end of 2037 (New York
// stays on standard time), which the port reproduces so results match.
const lastTransitionYear = 2037

var frozenFrom = time.Date(lastTransitionYear+1, 1, 1, 0, 0, 0, 0, time.UTC).Unix()

// offsetAt is the zone's UTC offset in seconds at a Unix time, as chrono-tz sees it.
func offsetAt(loc *time.Location, unix int64) int {
	if unix >= frozenFrom {
		unix = frozenFrom - 1
	}
	_, offset := time.Unix(unix, 0).In(loc).Zone()
	return offset
}

// wallClock is the zone's local date and time at a Unix time, as a UTC-labelled value.
func wallClock(loc *time.Location, unix int64) time.Time {
	return time.Unix(unix+int64(offsetAt(loc, unix)), 0).UTC()
}

// localTimes returns every Unix time at which the wall clock in loc reads the
// given fields: none inside a DST gap, two inside an overlap (earlier first).
func localTimes(loc *time.Location, year, month, day, hour, minute, second uint32) []int64 {
	wall := time.Date(int(year), time.Month(month), int(day), int(hour), int(minute), int(second), 0, time.UTC).Unix()
	offsets := map[int]bool{}
	for _, probe := range []int64{wall - 86400, wall, wall + 86400} {
		offsets[offsetAt(loc, probe)] = true
	}
	var found []int64
	for offset := range offsets {
		instant := wall - int64(offset)
		if offsetAt(loc, instant) == offset {
			found = append(found, instant)
		}
	}
	if len(found) == 2 && found[1] < found[0] {
		found[0], found[1] = found[1], found[0]
	}
	return found
}

// nextAfter is Schedule::next_after: the search is a literal port, including
// its skip of gap seconds and its reset bookkeeping. Times are Unix seconds;
// the crate ignores the sub-second part of the anchor.
func (s *Schedule) nextAfter(afterUnix int64, loc *time.Location) []int64 {
	query := newSearchState(wallClock(loc, afterUnix+1))
	afterYear := uint32(wallClock(loc, afterUnix).Year())
	for _, year := range s.years.within(uint32(query.initial.Year()), yearsUnit.max) {
		if year > afterYear {
			query.resetMonth()
		}
		monthStart := query.monthLowerBound()
		if !s.months.contains(monthStart) {
			query.resetMonth()
		}
		for _, month := range s.months.within(monthStart, monthsUnit.max) {
			if found := s.searchMonth(query, loc, year, month); found != nil {
				return found
			}
			query.resetMonth()
		}
	}
	return nil
}

func (s *Schedule) searchMonth(query *searchState, loc *time.Location, year, month uint32) []int64 {
	dayStart := query.dayLowerBound()
	if !s.daysOfMonth.contains(dayStart) {
		query.resetDay()
	}
	dayEnd := daysInMonth(month, year)
	var days []uint32
	for _, day := range s.daysOfMonth.within(min(dayStart, dayEnd), dayEnd) {
		if s.daysOfWeekIsAll() || s.daysOfWeek.contains(weekdayFromSunday(year, month, day)) {
			days = append(days, day)
		}
	}
	if len(days) == 0 || days[0] != dayStart {
		query.resetDay()
	}
	for _, day := range days {
		if found := s.searchDay(query, loc, year, month, day); found != nil {
			return found
		}
		query.resetDay()
	}
	return nil
}

func (s *Schedule) searchDay(query *searchState, loc *time.Location, year, month, day uint32) []int64 {
	hourStart := query.hourLowerBound()
	if !s.hours.contains(hourStart) {
		query.resetHour()
	}
	for _, hour := range s.hours.within(hourStart, hoursUnit.max) {
		if found := s.searchHour(query, loc, [3]uint32{year, month, day}, hour); found != nil {
			return found
		}
		query.resetHour()
	}
	return nil
}

func (s *Schedule) searchHour(query *searchState, loc *time.Location, date [3]uint32, hour uint32) []int64 {
	minuteStart := query.minuteLowerBound()
	if !s.minutes.contains(minuteStart) {
		query.resetMinute()
	}
	for _, minute := range s.minutes.within(minuteStart, minutesUnit.max) {
		secondStart := query.secondLowerBound()
		if !s.seconds.contains(secondStart) {
			query.resetSecond()
		}
		for _, second := range s.seconds.within(secondStart, secondsUnit.max) {
			if found := localTimes(loc, date[0], date[1], date[2], hour, minute, second); len(found) > 0 {
				return found
			}
		}
		query.resetMinute()
	}
	return nil
}

// Upcoming returns the next count triggers after anchor in loc, as the crate's
// iterator yields them: an ambiguous local time yields both instants.
func (s *Schedule) Upcoming(anchor time.Time, loc *time.Location, count int) []time.Time {
	var out []time.Time
	previous := anchor.Unix()
	var pendingLater *int64
	for len(out) < count {
		if pendingLater != nil {
			previous = *pendingLater
			out = append(out, time.Unix(previous, 0).In(loc))
			pendingLater = nil
			continue
		}
		found := s.nextAfter(previous, loc)
		if len(found) == 0 {
			return out
		}
		previous = found[0]
		out = append(out, time.Unix(previous, 0).In(loc))
		if len(found) == 2 {
			pendingLater = &found[1]
		}
	}
	return out
}

// After is the first trigger strictly after anchor (ignoring the sub-second
// part like the crate), or false when there is none before year 2100.
func (s *Schedule) After(anchor time.Time, loc *time.Location) (time.Time, bool) {
	found := s.nextAfter(anchor.Unix(), loc)
	if len(found) == 0 {
		return time.Time{}, false
	}
	return time.Unix(found[0], 0).In(loc), true
}
