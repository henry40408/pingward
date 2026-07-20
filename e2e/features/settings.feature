Feature: Settings

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: The settings page loads with empty defaults
    When I visit "/admin"
    Then the settings field "scan_interval" shows ""
    And the settings field "nag_interval" shows ""
    And the settings field "pings_retention_days" shows ""
    And the settings field "notifications_retention_days" shows ""

  Scenario: Saving settings persists them across a reload
    When I visit "/admin"
    And I fill the settings field "scan_interval" with "45"
    And I fill the settings field "nag_interval" with "600"
    And I fill the settings field "pings_retention_days" with "30"
    And I fill the settings field "notifications_retention_days" with "14"
    And I save the settings form
    Then the settings field "scan_interval" shows "45s"
    And the settings field "nag_interval" shows "10m"
    And the settings field "pings_retention_days" shows "30"
    And the settings field "notifications_retention_days" shows "14"

  Scenario: Blanking a saved setting clears it
    When I visit "/admin"
    And I fill the settings field "scan_interval" with "45"
    And I save the settings form
    And I fill the settings field "scan_interval" with ""
    And I save the settings form
    Then the settings field "scan_interval" shows ""

  Scenario: An invalid setting is rejected, preserved on the form, and not persisted
    When I visit "/admin"
    And I fill the settings field "scan_interval" with "99"
    And I fill the settings field "nag_interval" with "abc"
    And I save the settings form
    Then the settings form shows the error "Global nag interval must be a positive duration (e.g. 30, 5m, 1h30m)"
    And the settings field "scan_interval" shows "99"
    And the settings field "nag_interval" shows "abc"
    And the settings page shows no flash
    When I visit "/admin"
    Then the settings field "scan_interval" shows ""

  Scenario: Saving settings shows a one-shot confirmation flash
    When I visit "/admin"
    And I fill the settings field "scan_interval" with "45"
    And I save the settings form
    Then the settings page shows the flash "Settings saved."
    When I visit "/admin"
    Then the settings page shows no flash

  Scenario: Retention is a plain day count, not a duration
    When I visit "/admin"
    And I fill the settings field "pings_retention_days" with "5m"
    And I save the settings form
    Then the settings form shows the error "Pings retention must be a positive integer"
