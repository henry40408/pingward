@nojs
Feature: The UI with JavaScript switched off

  The `@nojs` tag is what opens this file's sessions with scripting disabled
  (`Emulation.setScriptExecutionDisabled`), and it is the only context in the
  suite that runs that way. Everything the rest of the suite exercises is seen
  through a browser running `app.js`, which makes it structurally blind to
  anything the UI has quietly started depending on script for.

  The rule these scenarios encode: `app.js` may make the UI nicer, never
  possible. Anything it is the sole provider of is a bug, whatever it looks
  like with script on.

  Background:
    Given an admin "admin" with password "correct horse battery" exists
    And I am signed in as "admin" with password "correct horse battery"
    And a project named "Nightly jobs"

  # The captured output is the single most useful thing on the page when a job
  # has broken, and it sat behind a caret that only `app.js` could open.
  Scenario: A failed job's captured output is readable
    Given a check named "backup" with period 60
    When I send a failing ping with output "boom: disk full"
    And I reload the check page
    Then the captured output "boom: disk full" is visible

  # A caret that cannot be clicked should not be drawn as though it can.
  Scenario: The rows advertise no affordance they cannot honour
    Given a check named "backup" with period 60
    When I send a failing ping with output "boom: disk full"
    And I reload the check page
    Then the expand carets are invisible

  # #143: the row was a div whose only route to the check page was a delegated
  # click handler, so an unscripted dashboard led nowhere at all.
  Scenario: A dashboard row reaches its check
    Given a check named "backup" with period 60
    When I visit the dashboard
    And I click the dashboard check link for "backup"
    Then I am on the check page

  # The filter forms had no method, no action, no field names and a
  # `type="button"` submit — four separate reasons a scriptless click did
  # nothing at all.
  Scenario: The pings filter narrows the table
    Given a check named "backup" with period 60
    When I send a "success" ping
    And I send a "fail" ping
    And I reload the check page
    And I filter the pings by kind "fail"
    Then the pings table shows 1 rows
    And the pings kind filter shows "fail"

  # A GET submission replaces the whole query string, so without the hidden
  # fields narrowing one section would silently clear the other's filter.
  Scenario: Filtering one section keeps the other section's filter
    Given a check named "backup" with period 60
    When I send a "success" ping
    And I send a "fail" ping
    And I reload the check page
    And I filter the notifications by event "down"
    And I filter the pings by kind "fail"
    # The row count is what proves a round trip happened at all: a select whose
    # value was merely set in the DOM would satisfy the two assertions below
    # without anything ever reaching the server.
    Then the pings table shows 1 rows
    And the pings kind filter shows "fail"
    And the notifications event filter shows "down"

  # `data-theme` is only ever set by `theme-init.js`, so every scriptless
  # visitor got the dark base regardless of what their OS asked for —
  # `prefers-color-scheme` needs no script to answer.
  Scenario: The OS colour scheme is honoured
    When my system prefers "light"
    And I visit the dashboard
    Then the page background is light
    When my system prefers "dark"
    And I visit the dashboard
    Then the page background is dark

  # Copy, LIVE and the theme cycle are pure `app.js`. A button that ignores
  # every click is worse than one that was never drawn.
  Scenario: Controls that would do nothing are not drawn
    Given a check named "backup" with period 60
    Then the copy button is absent
    And the live tail toggle is absent
    And the theme toggle is absent

  # The age is the number an operator actually reads off these tiles, and it
  # was rendered as an empty div for `app.js` to fill in every second.
  Scenario: The scheduler heartbeat shows how long ago it ran
    When I open the admin dashboard
    Then the scheduler heartbeat shows an age

  # The check form used to show the period *and* cron fields at once, and the
  # channel form all six kinds stacked, because only `app.js` hid the
  # irrelevant ones. The `:has()` rules follow the select on their own.
  Scenario: The check form shows only the selected schedule kind
    When I start creating a check
    Then the period field is visible
    And the cron field is hidden
    When I choose the "cron" schedule kind
    Then the cron field is visible
    And the period field is hidden

  # #144: `data-confirm` is inert without script, so the server asks instead.
  Scenario: Deleting a check asks first, as a page
    Given a check named "backup" with period 60
    When I click the delete check button
    Then the confirmation page asks about deleting
    When I confirm the pending action
    Then I am on the project page for "Nightly jobs"
