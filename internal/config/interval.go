package config

// EffectiveScanInterval picks the scan interval through check, project and
// global overrides. A value of zero or less falls through; the environment
// default is clamped to at least one second so the timer stays valid.
func EffectiveScanInterval(check, project, global *int64, envDefault int64) int64 {
	if seconds, ok := firstPositive(check, project, global); ok {
		return seconds
	}
	return max(envDefault, 1)
}

// EffectiveNagInterval picks the nag interval through check, project and
// global overrides; ok is false when none is positive. Nag is opt-in, so
// there is no environment default.
func EffectiveNagInterval(check, project, global *int64) (seconds int64, ok bool) {
	return firstPositive(check, project, global)
}

func firstPositive(overrides ...*int64) (int64, bool) {
	for _, override := range overrides {
		if override != nil && *override > 0 {
			return *override, true
		}
	}
	return 0, false
}
