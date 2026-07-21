Feature: Live tail

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"
    And a project named "Nightly jobs"
    And a check named "backup" with period 60

  Scenario: A ping appears without reloading while the live tail is on
    When I turn on the live tail
    And I send a "success" ping
    Then the recent pings table shows a "success" ping

  Scenario: Without the live tail a new ping only appears after a reload
    When I send a "success" ping
    Then the recent pings table still shows no pings
    When I reload the check page
    Then the recent pings table shows a "success" ping
