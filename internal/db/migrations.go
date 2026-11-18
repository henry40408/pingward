package db

import "embed"

// Embedded: the release image ships only the binary, with no migration files
// on disk. The files are checksummed byte for byte (see .gitattributes), so
// they must never be reformatted.
//
//go:embed migrations/sqlite/*.sql migrations/postgres/*.sql
var migrationFiles embed.FS
