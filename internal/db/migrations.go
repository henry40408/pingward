package db

import "embed"

// Embedded: the release image ships only the binary, with no migration files
// on disk.
//
//go:embed migrations/sqlite/*.sql migrations/postgres/*.sql
var migrationFiles embed.FS
