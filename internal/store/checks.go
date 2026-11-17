package store

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

type NewCheck struct {
	ProjectID        int64
	Name             string
	Description      string
	PingUUID         string
	Kind             models.ScheduleKind
	PeriodSecs       *int64
	GraceSecs        int64
	CronExpr         *string
	Timezone         string
	ScanIntervalSecs *int64
	MaxRuntimeSecs   *int64
	NagIntervalSecs  *int64
}

// UpdateCheck carries every editable field: on an UPDATE an unset field would
// overwrite the stored value (for example blank the name).
type UpdateCheck struct {
	Name             string
	Description      string
	Kind             models.ScheduleKind
	PeriodSecs       *int64
	GraceSecs        int64
	CronExpr         *string
	Timezone         string
	ScanIntervalSecs *int64
	MaxRuntimeSecs   *int64
	NagIntervalSecs  *int64
}

// PingMark is which columns of checks a ping rewrites; nil timestamps keep the
// stored value (see MarkPing), except NextDueAt, which is always written.
type PingMark struct {
	Status      models.CheckStatus
	LastPingAt  *time.Time
	LastStartAt *time.Time
	NextDueAt   *time.Time
}

// placeholders is "$from,$from+1,..." for count values.
func placeholders(count, from int) string {
	parts := make([]string, count)
	for i := range parts {
		parts[i] = fmt.Sprintf("$%d", from+i)
	}
	return strings.Join(parts, ",")
}

func int64Args(values []int64) []any {
	args := make([]any, len(values))
	for i, value := range values {
		args[i] = value
	}
	return args
}

func (s *Store) findCheck(ctx context.Context, query string, arg any) (*models.Check, error) {
	rows, err := s.DB.QueryContext(ctx, query, arg)
	if err != nil {
		return nil, err
	}
	check, found, err := readOptionalRow(rows, decodeCheck)
	if err != nil || !found {
		return nil, err
	}
	return &check, nil
}

// FindCheckByUUID returns nil when no check has the ping uuid.
func (s *Store) FindCheckByUUID(ctx context.Context, uuid string) (*models.Check, error) {
	return s.findCheck(ctx, "SELECT * FROM checks WHERE ping_uuid = $1", uuid)
}

// FindCheck returns nil when no check has the id.
func (s *Store) FindCheck(ctx context.Context, id int64) (*models.Check, error) {
	return s.findCheck(ctx, "SELECT * FROM checks WHERE id = $1", id)
}

// ListActiveChecks returns the checks the scanner watches. Corrupt rows are
// logged and skipped so one cannot abort the whole scan.
func (s *Store) ListActiveChecks(ctx context.Context) ([]models.Check, error) {
	return s.listChecksSkippingCorrupt(ctx, "SELECT * FROM checks WHERE status IN ('new','up')")
}

// ListDownChecks returns the nag candidates; corrupt rows are skipped as in
// ListActiveChecks.
func (s *Store) ListDownChecks(ctx context.Context) ([]models.Check, error) {
	return s.listChecksSkippingCorrupt(ctx, "SELECT * FROM checks WHERE status = 'down'")
}

func (s *Store) listChecksSkippingCorrupt(ctx context.Context, query string) ([]models.Check, error) {
	rows, err := s.DB.QueryContext(ctx, query)
	if err != nil {
		return nil, err
	}
	return readRows(rows, func(r *row) (models.Check, error) {
		check, err := decodeCheck(r)
		if err != nil {
			s.Logger.Error("skipping corrupt checks row", "id", r.values["id"], "error", err)
			return models.Check{}, errSkipRow
		}
		return check, nil
	})
}

func (s *Store) CreateCheck(ctx context.Context, check NewCheck, now time.Time) (int64, error) {
	var id int64
	err := s.DB.QueryRowContext(ctx,
		"INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, "+
			"grace_secs, cron_expr, timezone, scan_interval_secs, max_runtime_secs, "+
			"nag_interval_secs, status, created_at) VALUES "+
			"($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'new',$13) RETURNING id",
		check.ProjectID, check.Name, check.Description, check.PingUUID, string(check.Kind),
		check.PeriodSecs, check.GraceSecs, check.CronExpr, check.Timezone, check.ScanIntervalSecs,
		check.MaxRuntimeSecs, check.NagIntervalSecs, timeText(now)).Scan(&id)
	return id, err
}

func (s *Store) ListChecksForProject(ctx context.Context, projectID int64) ([]models.Check, error) {
	rows, err := s.DB.QueryContext(ctx, "SELECT * FROM checks WHERE project_id = $1 ORDER BY id", projectID)
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodeCheck)
}

// ListChecksForProjects is the batched ListChecksForProject, keyed by
// project id (id order kept; projects with no checks absent).
func (s *Store) ListChecksForProjects(ctx context.Context, projectIDs []int64) (map[int64][]models.Check, error) {
	byProject := map[int64][]models.Check{}
	if len(projectIDs) == 0 {
		return byProject, nil
	}
	rows, err := s.DB.QueryContext(ctx,
		"SELECT * FROM checks WHERE project_id IN ("+placeholders(len(projectIDs), 1)+") ORDER BY project_id, id",
		int64Args(projectIDs)...)
	if err != nil {
		return nil, err
	}
	checks, err := readRows(rows, decodeCheck)
	if err != nil {
		return nil, err
	}
	for _, check := range checks {
		byProject[check.ProjectID] = append(byProject[check.ProjectID], check)
	}
	return byProject, nil
}

func (s *Store) UpdateCheckSchedule(ctx context.Context, id int64, check UpdateCheck) error {
	_, err := s.DB.ExecContext(ctx,
		"UPDATE checks SET name=$1, description=$2, schedule_kind=$3, period_secs=$4, grace_secs=$5, "+
			"cron_expr=$6, timezone=$7, scan_interval_secs=$8, max_runtime_secs=$9, "+
			"nag_interval_secs=$10 WHERE id=$11",
		check.Name, check.Description, string(check.Kind), check.PeriodSecs, check.GraceSecs,
		check.CronExpr, check.Timezone, check.ScanIntervalSecs, check.MaxRuntimeSecs,
		check.NagIntervalSecs, id)
	return err
}

func (s *Store) RegenerateUUID(ctx context.Context, id int64, newUUID string) error {
	_, err := s.DB.ExecContext(ctx, "UPDATE checks SET ping_uuid = $1 WHERE id = $2", newUUID, id)
	return err
}

func (s *Store) DeleteCheck(ctx context.Context, id int64) error {
	_, err := s.DB.ExecContext(ctx, "DELETE FROM checks WHERE id = $1", id)
	return err
}

func (s *Store) MarkPing(ctx context.Context, checkID int64, mark PingMark) error {
	_, err := s.DB.ExecContext(ctx,
		"UPDATE checks SET status=$1, last_ping_at=COALESCE($2, last_ping_at), "+
			"last_start_at=COALESCE($3, last_start_at), next_due_at=$4 WHERE id=$5",
		string(mark.Status), optionalTimeText(mark.LastPingAt), optionalTimeText(mark.LastStartAt),
		optionalTimeText(mark.NextDueAt), checkID)
	return err
}

func (s *Store) SetStatus(ctx context.Context, checkID int64, status models.CheckStatus) error {
	_, err := s.DB.ExecContext(ctx, "UPDATE checks SET status=$1 WHERE id=$2", string(status), checkID)
	return err
}

// BeginDownAlert starts a down incident: it stamps the alert baseline and
// clears any prior acknowledgement so a fresh incident is never silent.
func (s *Store) BeginDownAlert(ctx context.Context, checkID int64, at time.Time) error {
	_, err := s.DB.ExecContext(ctx, "UPDATE checks SET last_alert_at=$1, acknowledged=0 WHERE id=$2",
		timeText(at), checkID)
	return err
}

// RecordReminder advances the alert baseline after emitting a reminder.
func (s *Store) RecordReminder(ctx context.Context, checkID int64, at time.Time) error {
	_, err := s.DB.ExecContext(ctx, "UPDATE checks SET last_alert_at=$1 WHERE id=$2", timeText(at), checkID)
	return err
}

// ClearNag clears nag state on recovery.
func (s *Store) ClearNag(ctx context.Context, checkID int64) error {
	_, err := s.DB.ExecContext(ctx, "UPDATE checks SET acknowledged=0, last_alert_at=NULL WHERE id=$1", checkID)
	return err
}

// Acknowledge silences reminders for the current down incident.
func (s *Store) Acknowledge(ctx context.Context, checkID int64) error {
	_, err := s.DB.ExecContext(ctx, "UPDATE checks SET acknowledged=1 WHERE id=$1", checkID)
	return err
}
