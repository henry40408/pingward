// Command pingward is a self-hosted cron-job and scheduled-task monitor.
package main

import (
	"io"
	"os"
)

// process is everything run needs from the outside world.
type process struct {
	lookupEnv func(string) (string, bool)
	stdout    io.Writer
	stderr    io.Writer
}

func main() {
	os.Exit(run(process{
		lookupEnv: os.LookupEnv,
		stdout:    os.Stdout,
		stderr:    os.Stderr,
	}))
}

// run returns the exit code.
func run(process) int {
	return 0
}
