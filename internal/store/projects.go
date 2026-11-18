package store

import (
	"context"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

// ProjectSettings are the editable fields of a project, shared by create and update.
type ProjectSettings struct {
	Name             string
	Description      string
	ScanIntervalSecs *int64
	NagIntervalSecs  *int64
}

// ProjectWithOwner is a project and its owner's username.
type ProjectWithOwner struct {
	Project       models.Project
	OwnerUsername string
}

func (s *Store) CreateProject(ctx context.Context, userID int64, project ProjectSettings, now time.Time) (int64, error) {
	var id int64
	err := s.DB.QueryRowContext(ctx,
		"INSERT INTO projects (user_id, name, description, scan_interval_secs, nag_interval_secs, created_at) "+
			"VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
		userID, project.Name, project.Description, project.ScanIntervalSecs, project.NagIntervalSecs,
		timeText(now)).Scan(&id)
	return id, err
}

// FindProject returns nil when no project has the id.
func (s *Store) FindProject(ctx context.Context, id int64) (*models.Project, error) {
	rows, err := s.DB.QueryContext(ctx, "SELECT * FROM projects WHERE id = $1", id)
	if err != nil {
		return nil, err
	}
	project, found, err := readOptionalRow(rows, decodeProject)
	if err != nil || !found {
		return nil, err
	}
	return &project, nil
}

// ListAllProjectsWithOwner returns every project with its owner's username, by project id.
func (s *Store) ListAllProjectsWithOwner(ctx context.Context) ([]ProjectWithOwner, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT p.*, u.username AS owner_username "+
			"FROM projects p JOIN users u ON u.id = p.user_id ORDER BY p.id")
	if err != nil {
		return nil, err
	}
	return readRows(rows, func(r *row) (ProjectWithOwner, error) {
		project, err := decodeProject(r)
		return ProjectWithOwner{Project: project, OwnerUsername: r.text("owner_username")}, err
	})
}

func (s *Store) ListProjectsForUser(ctx context.Context, userID int64) ([]models.Project, error) {
	rows, err := s.DB.QueryContext(ctx, "SELECT * FROM projects WHERE user_id = $1 ORDER BY id", userID)
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodeProject)
}

func (s *Store) UpdateProject(ctx context.Context, id int64, project ProjectSettings) error {
	_, err := s.DB.ExecContext(ctx,
		"UPDATE projects SET name = $1, description = $2, scan_interval_secs = $3, nag_interval_secs = $4 WHERE id = $5",
		project.Name, project.Description, project.ScanIntervalSecs, project.NagIntervalSecs, id)
	return err
}

func (s *Store) DeleteProject(ctx context.Context, id int64) error {
	_, err := s.DB.ExecContext(ctx, "DELETE FROM projects WHERE id = $1", id)
	return err
}

func (s *Store) AllProjectScanIntervals(ctx context.Context) (map[int64]*int64, error) {
	return s.projectIntervals(ctx, "SELECT id, scan_interval_secs AS secs FROM projects")
}

func (s *Store) AllProjectNagIntervals(ctx context.Context) (map[int64]*int64, error) {
	return s.projectIntervals(ctx, "SELECT id, nag_interval_secs AS secs FROM projects")
}

func (s *Store) projectIntervals(ctx context.Context, query string) (map[int64]*int64, error) {
	rows, err := s.DB.QueryContext(ctx, query)
	if err != nil {
		return nil, err
	}
	pairs, err := readRows(rows, func(r *row) ([2]any, error) {
		return [2]any{r.int64("id"), r.optionalInt64("secs")}, r.err
	})
	if err != nil {
		return nil, err
	}
	intervals := make(map[int64]*int64, len(pairs))
	for _, pair := range pairs {
		intervals[pair[0].(int64)] = pair[1].(*int64)
	}
	return intervals, nil
}

// DisplayTimezone is the instance-wide notification timezone; "" when unset or
// blank. A read failure also yields "": it must not stop a down alert going out.
func (s *Store) DisplayTimezone(ctx context.Context) string {
	value, err := s.GetSetting(ctx, "display_timezone")
	if err != nil || value == nil {
		return ""
	}
	return strings.TrimSpace(*value)
}

// AllProjectNames is one map per scan/nag pass, so naming projects costs a
// fixed query count.
func (s *Store) AllProjectNames(ctx context.Context) (map[int64]string, error) {
	rows, err := s.DB.QueryContext(ctx, "SELECT id, name FROM projects")
	if err != nil {
		return nil, err
	}
	names := map[int64]string{}
	_, err = readRows(rows, func(r *row) (struct{}, error) {
		names[r.int64("id")] = r.text("name")
		return struct{}{}, r.err
	})
	return names, err
}
