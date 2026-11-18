package store

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

// CursorKind says which side of the cursor id a page lies on.
type CursorKind int

const (
	CursorLatest CursorKind = iota // the newest page
	CursorBefore                   // rows older than ID
	CursorAfter                    // rows newer than ID
)

// PageCursor is a keyset cursor over id rather than created_at: monotonic,
// indexed, and stable under concurrent inserts. The zero value is the newest page.
type PageCursor struct {
	Kind CursorKind
	ID   int64
}

func Before(id int64) PageCursor { return PageCursor{Kind: CursorBefore, ID: id} }
func After(id int64) PageCursor  { return PageCursor{Kind: CursorAfter, ID: id} }

// Page is always newest-first (id DESC), whichever direction was queried.
type Page[T any] struct {
	Items    []T
	HasNewer bool
	HasOlder bool
}

// predicate is a filter condition: column and operator are caller literals,
// values are bound.
type predicate struct {
	column   string
	operator string // "" for an IN list
	values   []string
}

func compare(column, operator, value string) predicate {
	return predicate{column: column, operator: operator, values: []string{value}}
}

func in(column string, values []string) predicate {
	return predicate{column: column, values: values}
}

// dateBounds are re-serialised like the stored text so comparing them to
// created_at stays chronological.
func dateBounds(from, to *time.Time) []predicate {
	var out []predicate
	if from != nil {
		out = append(out, compare("created_at", ">=", timeText(*from)))
	}
	if to != nil {
		out = append(out, compare("created_at", "<=", timeText(*to)))
	}
	return out
}

// PingFilter: empty fields mean no constraint.
type PingFilter struct {
	Kinds []models.PingKind
	From  *time.Time
	To    *time.Time
}

func (f PingFilter) IsEmpty() bool {
	return len(f.Kinds) == 0 && f.From == nil && f.To == nil
}

func (f PingFilter) predicates() []predicate {
	var out []predicate
	if len(f.Kinds) > 0 {
		values := make([]string, len(f.Kinds))
		for i, kind := range f.Kinds {
			values[i] = string(kind)
		}
		out = append(out, in("kind", values))
	}
	return append(out, dateBounds(f.From, f.To)...)
}

// NotifFilter: empty fields mean no constraint.
type NotifFilter struct {
	Events   []models.EventKind
	Statuses []models.NotifyStatus
	From     *time.Time
	To       *time.Time
}

func (f NotifFilter) IsEmpty() bool {
	return len(f.Events) == 0 && len(f.Statuses) == 0 && f.From == nil && f.To == nil
}

func (f NotifFilter) predicates() []predicate {
	var out []predicate
	if len(f.Events) > 0 {
		values := make([]string, len(f.Events))
		for i, event := range f.Events {
			values[i] = string(event)
		}
		out = append(out, in("event", values))
	}
	if len(f.Statuses) > 0 {
		values := make([]string, len(f.Statuses))
		for i, status := range f.Statuses {
			values[i] = string(status)
		}
		out = append(out, in("status", values))
	}
	return append(out, dateBounds(f.From, f.To)...)
}

// AuditFilter: Actor and Action are exact matches. nil means no constraint;
// a pointer to "" matches only an empty value.
type AuditFilter struct {
	Actor  *string
	Action *string
	From   *time.Time
	To     *time.Time
}

func (f AuditFilter) IsEmpty() bool {
	return f.Actor == nil && f.Action == nil && f.From == nil && f.To == nil
}

func (f AuditFilter) predicates() []predicate {
	var out []predicate
	if f.Actor != nil {
		out = append(out, compare("actor_username", "=", *f.Actor))
	}
	if f.Action != nil {
		out = append(out, compare("action", "=", *f.Action))
	}
	return append(out, dateBounds(f.From, f.To)...)
}

type keysetQuery struct {
	table      string
	scopeCol   string // "" for no scope
	scopeID    int64
	cursor     PageCursor
	limit      int64
	predicates []predicate
}

// keysetSQL is the statement and its binds. Only literals are interpolated.
func keysetSQL(q keysetQuery) (string, []any) {
	var conds []string
	var binds []any
	bind := func(value any) string {
		binds = append(binds, value)
		return fmt.Sprintf("$%d", len(binds))
	}
	if q.scopeCol != "" {
		conds = append(conds, q.scopeCol+" = "+bind(q.scopeID))
	}
	for _, p := range q.predicates {
		if p.operator != "" {
			conds = append(conds, p.column+" "+p.operator+" "+bind(p.values[0]))
			continue
		}
		placeholders := make([]string, len(p.values))
		for i, value := range p.values {
			placeholders[i] = bind(value)
		}
		conds = append(conds, p.column+" IN ("+strings.Join(placeholders, ",")+")")
	}
	order := "DESC"
	switch q.cursor.Kind {
	case CursorBefore:
		conds = append(conds, "id < "+bind(q.cursor.ID))
	case CursorAfter:
		conds = append(conds, "id > "+bind(q.cursor.ID))
		order = "ASC"
	}
	limit := bind(q.limit + 1)
	where := ""
	if len(conds) > 0 {
		where = " WHERE " + strings.Join(conds, " AND ")
	}
	return "SELECT * FROM " + q.table + where + " ORDER BY id " + order + " LIMIT " + limit, binds
}

// keysetPage fetches limit+1 rows to detect another page without a COUNT(*).
func keysetPage[T any](ctx context.Context, s *Store, q keysetQuery, decode func(*row) (T, error)) (Page[T], error) {
	query, binds := keysetSQL(q)
	rows, err := s.DB.QueryContext(ctx, query, binds...)
	if err != nil {
		return Page[T]{}, err
	}
	items, err := readRows(rows, decode)
	if err != nil {
		return Page[T]{}, err
	}
	overflowed := int64(len(items)) > q.limit
	if q.cursor.Kind == CursorAfter {
		// ASC, so the overflow row is last (farthest from the cursor).
		if overflowed {
			items = items[:len(items)-1]
		}
		// A cursor came from an adjacent page, so the opposite flag is true.
		reverse(items)
		return Page[T]{Items: items, HasNewer: overflowed, HasOlder: true}, nil
	}
	if overflowed {
		items = items[:q.limit]
	}
	return Page[T]{Items: items, HasNewer: q.cursor.Kind == CursorBefore, HasOlder: overflowed}, nil
}

func reverse[T any](items []T) {
	for i, j := 0, len(items)-1; i < j; i, j = i+1, j-1 {
		items[i], items[j] = items[j], items[i]
	}
}

func (s *Store) ListAuditPage(ctx context.Context, cursor PageCursor, limit int64, filter AuditFilter) (Page[models.AuditLog], error) {
	return keysetPage(ctx, s, keysetQuery{
		table: "audit_log", cursor: cursor, limit: limit, predicates: filter.predicates(),
	}, decodeAudit)
}

func (s *Store) ListPingsPage(ctx context.Context, checkID int64, cursor PageCursor, limit int64, filter PingFilter) (Page[models.Ping], error) {
	return keysetPage(ctx, s, keysetQuery{
		table: "pings", scopeCol: "check_id", scopeID: checkID, cursor: cursor, limit: limit,
		predicates: filter.predicates(),
	}, decodePing)
}

func (s *Store) ListNotificationsPage(ctx context.Context, checkID int64, cursor PageCursor, limit int64, filter NotifFilter) (Page[models.Notification], error) {
	return keysetPage(ctx, s, keysetQuery{
		table: "notifications", scopeCol: "check_id", scopeID: checkID, cursor: cursor, limit: limit,
		predicates: filter.predicates(),
	}, decodeNotification)
}
