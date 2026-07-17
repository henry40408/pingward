@fast-scan
Feature: Time-dependent check states

  # These scenarios run pingward with a 1s scan interval (via the @fast-scan
  # tag) so the background scan loop downs overdue/overrun checks within a
  # couple of seconds — no ping drives the transition.

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: An overdue check is downed by the scan loop
    Given a project named "Ops"
    When I create a check that falls due almost immediately
    Then the check status eventually becomes down

  Scenario: An in-flight run over its max runtime is downed by the scan loop
    Given a project named "Ops"
    When I create a check with a 1 second max runtime
    And I send a "start" ping
    Then the check status eventually becomes down
