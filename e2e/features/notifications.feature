Feature: Notification channels

  Background:
    Given an admin "admin" with password "correct horse battery" exists
    And I am signed in as "admin" with password "correct horse battery"

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

  # The stored URL (a capability token) renders as a blank "unchanged" input,
  # so delivery still reaching the mock server is the proof a blank field kept it.
  Scenario: Renaming a channel keeps its stored webhook URL working
    Given a project named "Notify"
    And I remember the current project
    And a webhook channel named "hook1" targeting the mock server
    When I open the edit form for the channel "hook1"
    Then the edit form hides the stored webhook URL
    When I rename the channel to "hook-renamed"
    Then the project lists a channel named "hook-renamed" of kind "webhook"
    When I send a test notification to the channel "hook-renamed"
    Then a channel success banner is shown
    And the mock server receives a "test" notification

  # "hook1" starts at a dead port, so a successful delivery can only come from
  # the submitted URL.
  Scenario: Rotating a channel's webhook URL redirects delivery
    Given a project named "Notify"
    And I remember the current project
    And I create a webhook channel named "hook1"
    When I open the edit form for the channel "hook1"
    And I change the channel's webhook URL to the mock server
    And I send a test notification to the channel "hook1"
    Then a channel success banner is shown
    And the mock server receives a "test" notification

  Scenario: A channel's kind cannot be changed from the edit form
    Given a project named "Notify"
    And I remember the current project
    And I create a webhook channel named "hook1"
    When I open the edit form for the channel "hook1"
    Then the kind is shown as static text "webhook"

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
    And the "down" notification payload names project "Notify", links the check, and blames "failed"
    When I send a "success" ping
    Then the mock server receives a "up" notification
    And the check's recent notifications show a delivery to "hook1"
    And the recent notifications table shows a "down" event
    And the recent notifications table shows a "up" event

  # Check creation auto-binds the project's existing channels, so the
  # channels are created after "backup" to leave it unbound.
  Scenario: A check page shows explicit ON/OFF state per channel
    Given a project named "Notify"
    And I remember the current project
    And a check named "backup" with period 3600
    And I create a webhook channel named "hook-on"
    And I create a webhook channel named "hook-off"
    When I visit the check page for "backup"
    And I bind the channel "hook-on" to the check
    Then the channel "hook-on" shows as ON on the check page
    And the channel "hook-off" shows as OFF on the check page

  # Same auto-bind ordering: "solo" predates "hook1", "covered" follows it.
  Scenario: The dashboard flags a check with no notification channel
    Given a project named "Notify"
    And I remember the current project
    And a check named "solo" with period 3600
    And I create a webhook channel named "hook1"
    And I create a check named "covered" with period 3600
    When I visit the dashboard
    Then the dashboard shows a "no channel" chip for the check "solo"
    And the dashboard shows no "no channel" chip for the check "covered"
