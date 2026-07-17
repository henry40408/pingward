Feature: Check creation branches

  Beyond the period happy path in monitoring.feature, the new-check form
  supports a cron schedule and validates its inputs: server-side when the
  period or cron expression is missing, and client-side (a name is required).

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"
    And a project named "Nightly jobs"

  Scenario: Create a check on a cron schedule
    When I create a cron check named "hourly" with expression "0 0 * * * *"
    Then I am on the check page
    And the check status is "new"
    And the check schedule shows "0 0 * * * *"

  Scenario: The period and cron fields are never both shown
    Given I open the new check form
    Then only the period field is shown
    When I choose the "cron" schedule kind
    Then only the cron field is shown

  Scenario: The new check form requires a name
    Given I open the new check form
    When I fill the check period with 60
    And I submit the check form
    Then I am still on the new check form
    And the check name field is required

  Scenario: Creating a period check without a period is rejected
    Given I open the new check form
    When I fill the check name with "backup"
    And I submit the check form
    Then the check form shows the error "period_secs required for period mode"

  Scenario: Creating a cron check without an expression is rejected
    Given I open the new check form
    When I fill the check name with "backup"
    And I choose the "cron" schedule kind
    And I submit the check form
    Then the check form shows the error "cron_expr required for cron mode"

  Scenario: A human-readable period is accepted
    Given I open the new check form
    When I fill the check name with "backup"
    And I fill the check period with "1h30m"
    And I submit the check form
    Then I am on the check page
    And the check schedule shows "every 1h30m"

  Scenario: The project page shows a check's interval, not just its schedule kind
    When I create a check named "backup" with period 3600
    And I visit the project page for "Nightly jobs"
    Then the check schedule shows "every 1h · 5m grace"

  Scenario: A cron check's schedule includes its grace period
    When I create a cron check named "hourly" with expression "0 0 * * * *"
    Then I am on the check page
    And the check schedule shows "· 5m grace"
