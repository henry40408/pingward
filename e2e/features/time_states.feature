@fast-scan
Feature: Time-dependent check states

  # @fast-scan (1s scan interval) lets the scan loop, not a ping, down the
  # check within seconds.

  Background:
    Given an admin "admin" with password "correct horse battery" exists
    And I am signed in as "admin" with password "correct horse battery"

  Scenario: An overdue check is downed by the scan loop
    Given a project named "Ops"
    When I create a check that falls due almost immediately
    Then the check status eventually becomes down

  Scenario: An in-flight run over its max runtime is downed by the scan loop
    Given a project named "Ops"
    When I create a check with a 1 second max runtime
    And I send a "start" ping
    Then the check status eventually becomes down
