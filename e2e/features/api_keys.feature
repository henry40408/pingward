Feature: API keys

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: Create an API key and see the token exactly once
    When I open the API keys page
    And I create an API key named "CI deploy"
    Then the new API key token is shown once
    And the API keys list shows a key named "CI deploy"

  Scenario: Revoke an API key
    When I open the API keys page
    And I create an API key named "temp"
    And I revoke the API key
    Then no API keys remain
