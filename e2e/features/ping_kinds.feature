Feature: Ping kinds

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"
    And a project named "Nightly jobs"
    And a check named "backup" with period 60

  Scenario: A start ping is recorded and leaves the status unchanged
    When I send a "start" ping
    And I reload the check page
    Then the check status is "new"
    And the recent pings table shows a "start" ping

  Scenario: A log ping is recorded and leaves the status unchanged
    When I send a "log" ping
    And I reload the check page
    Then the check status is "new"
    And the recent pings table shows a "log" ping

  Scenario: An exit code of 0 marks the check up
    When I send an exit code 0 ping
    And I reload the check page
    Then the check status is "up"
    And the recent pings table shows the exit "exit 0"

  Scenario: A non-zero exit code marks the check down
    When I send an exit code 1 ping
    And I reload the check page
    Then the check status is "down"
    And the recent pings table shows the exit "exit 1"

  Scenario: Pinging an unknown UUID returns 404
    When I ping an unknown UUID
    Then the ping response status is 404

  Scenario: A POST body is captured and shown on the check page
    When I send a "success" ping with body "hello from cron"
    And I reload the check page
    And I expand the latest ping row
    Then the captured output shows "hello from cron"
