Feature: Account

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: The account page marks the current session
    When I open the account page
    Then the current session is marked as this device

  # The server is spawned with 127.0.0.1 trusted, so the forwarded header is
  # honoured; signing in again is what creates the session that records it.
  @trusted-proxy
  Scenario: A session behind a trusted proxy records the forwarded client IP
    Given requests arrive through a trusted proxy as "203.0.113.7"
    And I sign out
    And I am signed in as "admin" with password "correct horse"
    When I open the account page
    Then the current session shows the IP "203.0.113.7"

  Scenario: Revoking the current session signs you out
    When I open the account page
    And I revoke the current session
    Then I am on the login page

  Scenario: Create an API key and see the token exactly once
    When I open the account page
    And I create an API key named "CI deploy"
    Then the new API key token is shown once
    And the API keys list shows a key named "CI deploy"

  Scenario: Revoke an API key
    When I open the account page
    And I create an API key named "temp"
    And I revoke the API key
    Then no API keys remain
