Feature: The UI with JavaScript switched off

  Runs in the `no-js` project (playwright.config.js), the only context in the
  suite with scripting disabled. Everything the rest of the suite exercises is
  seen through a browser running `app.js`, which makes it structurally blind to
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

  # #144: `data-confirm` is inert without script, so the server asks instead.
  Scenario: Deleting a check asks first, as a page
    Given a check named "backup" with period 60
    When I click the delete check button
    Then the confirmation page asks about deleting
    When I confirm the pending action
    Then I am on the project page for "Nightly jobs"
