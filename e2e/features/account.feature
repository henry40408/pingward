Feature: Account

  Background:
    Given an admin "admin" with password "correct horse battery" exists
    And I am signed in as "admin" with password "correct horse battery"

  Scenario: The account page marks the current session
    When I open the account page
    Then the current session is marked as this device

  # The server is spawned with 127.0.0.1 trusted, so the forwarded header is
  # honoured; signing in again is what creates the session that records it.
  @trusted-proxy
  Scenario: A session behind a trusted proxy records the forwarded client IP
    Given requests arrive through a trusted proxy as "203.0.113.7"
    And I sign out
    And I am signed in as "admin" with password "correct horse battery"
    When I open the account page
    Then the current session shows the IP "203.0.113.7"

  Scenario: Revoking the current session signs you out
    When I open the account page
    And I revoke the current session
    Then I am on the login page

  Scenario: Change your own password and sign in with the new one
    When I open the account page
    And I change my password from "correct horse battery" to "battery staple horse"
    Then the password change is confirmed
    When I sign out
    And I sign in as "admin" with password "correct horse battery"
    Then the login page shows the error "invalid username or password"
    When I sign in as "admin" with password "battery staple horse"
    Then I land on the dashboard signed in

  Scenario: The wrong current password is refused
    When I open the account page
    And I change my password from "wrong" to "battery staple horse"
    Then the password change is rejected

  Scenario: Create an API key and see the token exactly once
    When I open the account page
    And I create an API key named "CI deploy" with my password "correct horse battery"
    Then the new API key token is shown once
    And the API keys list shows a key named "CI deploy"

  Scenario: Revoke an API key
    When I open the account page
    And I create an API key named "temp" with my password "correct horse battery"
    And I revoke the API key
    Then no API keys remain

  # An API key is not bound by the session caps and survives a password reset,
  # so minting one asks for the password again — a borrowed browser must not be
  # convertible into permanent access.
  Scenario: Creating an API key with the wrong password is refused
    When I open the account page
    And I create an API key named "CI deploy" with my password "not my password"
    Then the API key creation is rejected
    And no API keys remain
