Feature: Notification channels

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario Outline: Creating a <kind> channel lists it on the project
    Given a project named "Notify"
    And I remember the current project
    When I create a <kind> channel named "my-<kind>"
    Then the project lists a channel named "my-<kind>" of kind "<kind>"

    Examples:
      | kind     |
      | webhook  |
      | slack    |
      | telegram |
      | ntfy     |
      | pushover |

  Scenario: A webhook channel with a blank URL is rejected
    Given a project named "Notify"
    And I remember the current project
    When I submit a webhook channel with a blank URL
    Then the channel form shows an error "a webhook URL is required"

  Scenario: The email channel kind is not offered without instance SMTP
    Given a project named "Notify"
    And I remember the current project
    When I open the new channel form
    Then the "email" channel kind is not offered

  Scenario: Deleting a channel removes it from the project
    Given a project named "Notify"
    And I remember the current project
    And I create a webhook channel named "hook1"
    When I delete the channel named "hook1"
    Then the project shows no channels

  Scenario: A check whose project has no channels shows an empty state
    Given a project named "Notify"
    And a check named "backup" with period 3600
    Then the check's notify channels show an empty state

  Scenario: Binding a channel to a check persists
    Given a project named "Notify"
    And I remember the current project
    And I create a webhook channel named "hook1"
    And a check named "backup" with period 3600
    When I bind the channel "hook1" to the check
    Then the channel "hook1" is bound to the check
    And a "Notify channels saved." confirmation is shown
    And the confirmation is gone after reloading

  Scenario: A test notification to a reachable webhook succeeds
    Given a project named "Notify"
    And I remember the current project
    And a webhook channel named "hook1" targeting the mock server
    When I send a test notification to the channel "hook1"
    Then a channel success banner is shown
    And the mock server receives a "test" notification

  Scenario: A test notification to an unreachable webhook fails
    Given a project named "Notify"
    And I remember the current project
    And I create a webhook channel named "hook1"
    When I send a test notification to the channel "hook1"
    Then a channel error banner is shown

  Scenario: Down and up transitions deliver to a bound webhook
    Given a project named "Notify"
    And I remember the current project
    And a webhook channel named "hook1" targeting the mock server
    And a check named "backup" with period 3600
    And I bind the channel "hook1" to the check
    When I send a "fail" ping
    Then the mock server receives a "down" notification
    When I send a "success" ping
    Then the mock server receives a "up" notification
    And the check's recent notifications show a delivery to "hook1"
