Feature: Account

  Background:
    Given an admin "admin" with password "correct horse battery" exists
    And I am signed in as "admin" with password "correct horse battery"

  Scenario: The account page marks the current session
    When I open the account page
    Then the current session is marked as this device

  # @trusted-proxy trusts 127.0.0.1; signing in again creates the session
  # that records the forwarded IP.
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

  # A key escapes session caps and survives a password reset, so minting one
  # re-asks the password.
  Scenario: Creating an API key with the wrong password is refused
    When I open the account page
    And I create an API key named "CI deploy" with my password "not my password"
    Then the API key creation is rejected
    And no API keys remain
