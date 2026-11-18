package e2e

import "github.com/cucumber/godog"

func registerAllSteps(sc *godog.ScenarioContext, w *world) {
	registerAccountSteps(sc, w)
	registerAssetsSteps(sc, w)
	registerAuthSteps(sc, w)
	registerAuthzSteps(sc, w)
	registerCheckCreateSteps(sc, w)
	registerCheckHistorySteps(sc, w)
	registerEditFlowSteps(sc, w)
	registerLiveTailSteps(sc, w)
	registerAdminSteps(sc, w)
	registerMobileLayoutSteps(sc, w)
	registerMonitoringSteps(sc, w)
	registerNoJSSteps(sc, w)
	registerNotificationsSteps(sc, w)
	registerPingKindsSteps(sc, w)
	registerSettingsSteps(sc, w)
	registerThemeSteps(sc, w)
	registerTimeStatesSteps(sc, w)
	registerUsersSteps(sc, w)
	registerValidationSteps(sc, w)
}
