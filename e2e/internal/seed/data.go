package seed

const (
	hour = 3600
	day  = 24 * hour
)

type state int

const (
	stateUp state = iota
	stateDown
	stateLate
	stateRunning
	statePaused
)

// extraUsers get /admin's cross-user cards some content.
var extraUsers = []struct {
	username string
	isAdmin  bool
}{{"maya", true}, {"sam", false}}

type project struct {
	key string
	// owner indexes into [demo, maya, sam].
	owner            int
	name             string
	description      string
	scanIntervalSecs *int64
	nagIntervalSecs  *int64
}

func secs(value int64) *int64 { return &value }

var projects = []project{
	{
		key: "backups", owner: 0, name: "Backups",
		description:     "Nightly database dumps and offsite sync. **Paging** goes to the on-call rotation.",
		nagIntervalSecs: secs(1800),
	},
	{
		key: "pipeline", owner: 0, name: "Data pipeline",
		description:      "Hourly ETL plus the downstream dbt models.",
		scanIntervalSecs: secs(60),
	},
	{
		key: "web", owner: 0, name: "Website",
		description: "Certificate renewal and the housekeeping crons for the public site.",
	},
	{
		key: "staging", owner: 2, name: "Staging (sam)",
		description: "Another user's project — only reachable from /admin.",
	},
}

type check struct {
	key         string
	project     string
	name        string
	description string
	cron        string // empty for a period check
	timezone    string
	// cadence is the run interval in seconds: it spaces the ping history and,
	// for period checks, is the schedule.
	cadence int64
	grace   int64
	// maxRuntime is 0 when unset.
	maxRuntime int64
	state      state
	// runtimeLow and runtimeHigh bound the duration band (secs); keep it a
	// visible share of maxRuntime, which the heartbeat scales bar height by.
	runtimeLow, runtimeHigh float64
	channels                []string
}

var checks = []check{
	{
		key: "pg-dump", project: "backups", name: "postgres-nightly-dump",
		description: "`pg_dump` of the primary, streamed straight to object storage.",
		cron:        "0 30 2 * * *", timezone: "Europe/Berlin",
		cadence: day, grace: 15 * 60, maxRuntime: 45 * 60, state: stateUp,
		runtimeLow: 900, runtimeHigh: 1500,
		channels: []string{"ops-slack", "oncall-pushover"},
	},
	{
		key: "s3-sync", project: "backups", name: "s3-offsite-sync",
		description: "Mirrors last night's dump to a second region.",
		timezone:    "UTC",
		cadence:     hour, grace: 5 * 60, maxRuntime: 20 * 60, state: stateUp,
		runtimeLow: 120, runtimeHigh: 900,
		channels: []string{"ops-slack"},
	},
	{
		key: "nas-snapshot", project: "backups", name: "home-nas-snapshot",
		description: "ZFS snapshot + scrub on the NAS. Missed its window last night.",
		timezone:    "UTC",
		cadence:     6 * hour, grace: 30 * 60, maxRuntime: 90 * 60, state: stateDown,
		runtimeLow: 1400, runtimeHigh: 3200,
		channels: []string{"ops-slack", "oncall-pushover", "alerts-email"},
	},
	{
		key: "archive-verify", project: "backups", name: "photo-archive-verify",
		description: "Weekly checksum sweep over the cold archive. Paused during the migration.",
		timezone:    "UTC",
		cadence:     7 * day, grace: day, state: statePaused,
		runtimeLow: 4000, runtimeHigh: 9000,
		channels: []string{"alerts-email"},
	},
	{
		key: "etl", project: "pipeline", name: "etl-hourly",
		description: "Extracts yesterday's events into the warehouse.",
		timezone:    "UTC",
		cadence:     hour, grace: 10 * 60, maxRuntime: 25 * 60, state: stateRunning,
		runtimeLow: 400, runtimeHigh: 1100,
		channels: []string{"pipeline-ntfy"},
	},
	{
		key: "dbt", project: "pipeline", name: "dbt-run",
		description: "Rebuilds the marts once the ETL lands.",
		cron:        "0 15 */4 * * *", timezone: "UTC",
		cadence: 4 * hour, grace: 10 * 60, maxRuntime: 30 * 60, state: stateUp,
		runtimeLow: 600, runtimeHigh: 1500,
		channels: []string{"pipeline-ntfy"},
	},
	{
		key: "feature-export", project: "pipeline", name: "feature-export",
		description: "Pushes the feature table to the serving store. Running behind.",
		timezone:    "UTC",
		cadence:     30 * 60, grace: 10 * 60, maxRuntime: 15 * 60, state: stateLate,
		runtimeLow: 200, runtimeHigh: 700,
		channels: []string{"pipeline-ntfy"},
	},
	{
		key: "certbot", project: "web", name: "certbot-renew",
		description: "Weekly ACME renewal for `www` and the wildcard.",
		cron:        "0 0 4 * * 1", timezone: "UTC",
		cadence: 7 * day, grace: 6 * hour, maxRuntime: 4 * 60, state: stateUp,
		runtimeLow: 40, runtimeHigh: 170,
		channels: []string{"ops-slack"},
	},
	{
		key: "sitemap", project: "web", name: "sitemap-rebuild",
		timezone: "UTC",
		cadence:  day, grace: 2 * hour, maxRuntime: 15 * 60, state: stateUp,
		runtimeLow: 180, runtimeHigh: 640,
	},
	{
		key: "linkcheck", project: "web", name: "broken-link-sweep",
		description: "Crawls the docs for dead links.",
		timezone:    "UTC",
		cadence:     12 * hour, grace: hour, maxRuntime: 40 * 60, state: stateUp,
		runtimeLow: 800, runtimeHigh: 2100,
		channels: []string{"ops-slack"},
	},
	{
		key: "staging-seed", project: "staging", name: "staging-db-reseed",
		timezone: "UTC",
		cadence:  day, grace: hour, state: stateUp,
		runtimeLow: 300, runtimeHigh: 800,
	},
}

type channel struct {
	key, project, kind, name, config string
}

var channels = []channel{
	{"ops-slack", "backups", "slack", "#ops-alerts",
		`{"url":"https://hooks.slack.com/services/T000/B000/xxxxxxxxxxxx"}`},
	{"oncall-pushover", "backups", "pushover", "on-call phone",
		`{"token":"axxxxxxxxxxxxxxxxxxxxxxxxxxxxx","user":"uxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}`},
	{"alerts-email", "backups", "email", "alerts@example.com",
		`{"to":"alerts@example.com"}`},
	{"pipeline-ntfy", "pipeline", "ntfy", "ntfy · data-pipeline",
		`{"base_url":"https://ntfy.sh","topic":"data-pipeline"}`},
}

var failBodies = []string{
	"rsync: connection unexpectedly closed (0 bytes received so far)\nrsync error: error in rsync protocol data stream (code 12) at io.c(228)",
	"pg_dump: error: connection to server failed: FATAL:  the database system is in recovery mode",
	"zpool status: one or more devices has experienced an unrecoverable error\n  scrub repaired 0B with 3 errors",
}

var sourceIPs = []string{"10.4.2.15", "10.4.2.31", "192.168.20.8"}

// runs is enough to fill the heartbeat strip.
const runs = 34

type audit struct {
	actor, action, targetType string
	targetID                  int
	method, path              string
	detail                    string // empty means NULL
	// ago is seconds before now.
	ago float64
}

// audits are shaped like real record_audit rows, oldest first: the card orders
// by id, which must agree with the timestamps.
var audits = []audit{
	{"demo", "user.create", "user", 3, "POST", "/admin/users", "username=sam is_admin=false", 30 * day},
	{"demo", "user.set_admin", "user", 2, "POST", "/admin/users/2/admin", "is_admin=true", 26 * day},
	{"maya", "user.password_reset", "user", 3, "POST", "/admin/users/3/password", "", 6 * day},
	{"maya", "admin.access", "project", 1, "GET", "/admin/projects/1", "", 2.5 * hour},
	{"maya", "admin.access", "check", 3, "GET", "/admin/checks/3", "", 2.4 * hour},
}

// settings: prune runs at boot. 90 days keeps all history except the weekly
// checks' oldest runs (34 weeks), which the screenshots never show.
var settings = [][2]string{
	{"scan_interval", "30"},
	{"nag_interval", "1800"},
	{"pings_retention_days", "90"},
	{"notifications_retention_days", "90"},
}
