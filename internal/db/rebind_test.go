package db

import (
	"context"
	"testing"
)

func TestRebind(t *testing.T) {
	cases := []struct{ name, in, want string }{
		{"plain", "SELECT $1, $2", "SELECT ?1, ?2"},
		{"reverse order", "SELECT $2, $1", "SELECT ?2, ?1"},
		{"reused", "SELECT $1 WHERE a = $1", "SELECT ?1 WHERE a = ?1"},
		{"two digits", "VALUES ($10, $11)", "VALUES (?10, ?11)"},
		{"string literal", "SELECT '$1', $1", "SELECT '$1', ?1"},
		{"escaped quote", "SELECT 'it''s $1', $1", "SELECT 'it''s $1', ?1"},
		{"quoted identifier", `SELECT "a$1", $1`, `SELECT "a$1", ?1`},
		{"line comment", "SELECT 1 -- $1\n, $1", "SELECT 1 -- $1\n, ?1"},
		{"block comment", "SELECT /* $1 */ $1", "SELECT /* $1 */ ?1"},
		{"unterminated string", "SELECT 'abc $1", "SELECT 'abc $1"},
		{"unterminated block comment", "SELECT /* $1", "SELECT /* $1"},
		{"dollar without digit", "SELECT $a, $", "SELECT $a, $"},
		{"no placeholder", "SELECT 1", "SELECT 1"},
		{"digit range ends", "SELECT $0, $9", "SELECT ?0, ?9"},
		{"characters next to the digit range", "SELECT $/, $:", "SELECT $/, $:"},
		{"empty string literal", "SELECT '', $1", "SELECT '', ?1"},
		{"query ends with a closing quote", "SELECT $1, 'a'", "SELECT ?1, 'a'"},
		{"empty block comment", "SELECT /**/ $1", "SELECT /**/ ?1"},
		{"block comment opened by a slash", "SELECT /*/ $1 */ $1", "SELECT /*/ $1 */ ?1"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := SQLite.Rebind(c.in); got != c.want {
				t.Errorf("got %q, want %q", got, c.want)
			}
			if got := Postgres.Rebind(c.in); got != c.in {
				t.Errorf("postgres must not rewrite: %q", got)
			}
		})
	}
}

func TestRebindMakesOutOfOrderAndReusedParametersBind(t *testing.T) {
	database := openMemory(t)
	var first, second, again int
	err := database.QueryRowContext(context.Background(), "SELECT $2, $1, $1", 10, 20).Scan(&first, &second, &again)
	if err != nil {
		t.Fatal(err)
	}
	if first != 20 || second != 10 || again != 10 {
		t.Fatalf("got %d %d %d", first, second, again)
	}
}

// Documents modernc's own behaviour: SQLite names "$1" as a parameter, and the
// driver maps such names back to ordinals. The rewrite to ?NNN does not depend
// on that mapping, which is why it stays.
func TestModerncBindsDollarNumbersNatively(t *testing.T) {
	database := openMemory(t)
	var first, second int
	err := database.DB.QueryRowContext(context.Background(), "SELECT $2, $1", 10, 20).Scan(&first, &second)
	if err != nil {
		t.Skipf("modernc no longer binds $N natively: %v", err)
	}
	if first != 20 || second != 10 {
		t.Fatalf("native $N binding changed: %d %d", first, second)
	}
}
