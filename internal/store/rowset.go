package store

import (
	"database/sql"
	"fmt"
	"time"
)

// row is one result row addressed by column name, so statements can keep
// "SELECT *" and aliases exactly as the Rust build wrote them. A column of an
// unexpected type is remembered in err instead of panicking.
type row struct {
	values map[string]any
	err    error
}

// readRows decodes every row with decode.
func readRows[T any](rows *sql.Rows, decode func(*row) (T, error)) ([]T, error) {
	defer rows.Close()
	columns, err := rows.Columns()
	if err != nil {
		return nil, err
	}
	var out []T
	for rows.Next() {
		current, err := scanRow(rows, columns)
		if err != nil {
			return nil, err
		}
		item, err := decode(current)
		if err != nil {
			return nil, err
		}
		out = append(out, item)
	}
	return out, rows.Err()
}

func scanRow(rows *sql.Rows, columns []string) (*row, error) {
	holders := make([]any, len(columns))
	targets := make([]any, len(columns))
	for i := range holders {
		targets[i] = &holders[i]
	}
	if err := rows.Scan(targets...); err != nil {
		return nil, err
	}
	values := make(map[string]any, len(columns))
	for i, name := range columns {
		values[name] = holders[i]
	}
	return &row{values: values}, nil
}

// readOptionalRow decodes the first row, or reports found=false.
func readOptionalRow[T any](rows *sql.Rows, decode func(*row) (T, error)) (item T, found bool, err error) {
	items, err := readRows(rows, decode)
	if err != nil || len(items) == 0 {
		return item, false, err
	}
	return items[0], true, nil
}

func (r *row) fail(column, want string, got any) {
	if r.err == nil {
		r.err = fmt.Errorf("column %q: want %s, got %T", column, want, got)
	}
}

func (r *row) int64(column string) int64 {
	switch value := r.values[column].(type) {
	case int64:
		return value
	case bool:
		return boolToInt(value)
	default:
		r.fail(column, "an integer", value)
		return 0
	}
}

func (r *row) bool(column string) bool { return r.int64(column) != 0 }

func (r *row) optionalInt64(column string) *int64 {
	if r.values[column] == nil {
		return nil
	}
	value := r.int64(column)
	return &value
}

func (r *row) text(column string) string {
	switch value := r.values[column].(type) {
	case string:
		return value
	case []byte:
		return string(value)
	default:
		r.fail(column, "text", value)
		return ""
	}
}

func (r *row) optionalText(column string) *string {
	if r.values[column] == nil {
		return nil
	}
	value := r.text(column)
	return &value
}

func (r *row) nullText(column string) sql.NullString {
	text := r.optionalText(column)
	if text == nil {
		return sql.NullString{}
	}
	return sql.NullString{String: *text, Valid: true}
}

// optionalTime is lenient (see parseStoredTime).
func (r *row) optionalTime(column string) *time.Time {
	return parseStoredTime(r.nullText(column))
}

// requiredTime fails the row when the column is not RFC 3339.
func (r *row) requiredTime(table, column string) time.Time {
	parsed := r.optionalTime(column)
	if parsed == nil {
		if r.err == nil {
			r.err = fmt.Errorf("%s.%s must be RFC3339", table, column)
		}
		return time.Time{}
	}
	return *parsed
}
