Feature: Monitoring core

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: Create a project
    When I create a project named "Nightly jobs"
    Then I am on the project page for "Nightly jobs"

  Scenario: Create a check
    Given a project named "Nightly jobs"
    When I create a check named "backup" with period 60
    Then I am on the check page
    And the check status is "new"
    And the ping URL is shown

  Scenario: A success ping turns the check up
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I send a "success" ping
    And I reload the check page
    Then the check status is "up"

  Scenario: A fail ping turns the check down
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I send a "fail" ping
    And I reload the check page
    Then the check status is "down"

  Scenario: Acknowledge a down check
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    And I send a "fail" ping
    And I reload the check page
    When I acknowledge the check
    Then the acknowledge control is gone

  Scenario: Pause and resume a check
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I pause the check
    Then the check status is "paused"
    When I resume the check
    Then the check status is not "paused"

  Scenario: A ping does not resurrect a paused check
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I pause the check
    And I send a "success" ping
    And I reload the check page
    Then the check status is "paused"

  Scenario: Regenerate the ping URL
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I regenerate the ping URL
    Then the ping URL is different from before

  Scenario: Delete a check
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I delete the check
    Then the project has no checks

  Scenario: Delete a project
    Given a project named "Nightly jobs"
    When I delete the project
    Then the dashboard shows no projects

  Scenario: The dashboard filter narrows the list and can be cleared
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    And a project named "Weekly jobs"
    And a check named "report" with period 60
    When I filter the dashboard by "report"
    Then the dashboard shows the check "report"
    And the dashboard does not show the check "backup"
    When I clear the dashboard filter
    Then the dashboard shows the check "backup"
    And the dashboard shows the check "report"

  Scenario: A dashboard filter that matches nothing says so
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I filter the dashboard by "nonesuch"
    Then the dashboard says nothing matched

  Scenario: A fresh check shows empty ping and notification tables
    Given a project named "Fresh"
    And a check named "newbie" with period 3600
    Then the recent pings table shows an empty state
    And the recent notifications table shows an empty state
