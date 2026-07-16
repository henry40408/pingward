Feature: Edit flows

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: Rename a project
    Given a project named "Nightly jobs"
    When I open the project edit form
    And I change the project name to "Daily jobs"
    Then I am on the project page for "Daily jobs"

  Scenario: Rename a check
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I open the check edit form
    And I change the check name to "database backup"
    Then I am on the check page
    And the check name is "database backup"

  Scenario: Change a check's period
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I open the check edit form
    And I change the check period to 300
    Then I am on the check page
    And the check schedule shows "every 5m 00s"

  Scenario: Change a check's grace period
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I open the check edit form
    And I change the check grace to 600
    Then I am on the check page
    And the check schedule shows "10m 00s grace"

  Scenario: Change a check's timezone
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I open the check edit form
    And I change the check timezone to "America/New_York"
    And I open the check edit form
    Then the check timezone field shows "America/New_York"
