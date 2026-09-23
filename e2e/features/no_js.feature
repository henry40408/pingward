@nojs
Feature: The UI with JavaScript switched off

  `@nojs` disables page scripts (`Emulation.setScriptExecutionDisabled`); this
  is the only file that runs that way, so the only place a hidden dependency on
  `app.js` shows up. The rule: `app.js` may make the UI nicer, never possible.

  Background:
    Given an admin "admin" with password "correct horse battery" exists
    And I am signed in as "admin" with password "correct horse battery"
    And a project named "Nightly jobs"

  # Output panels collapse only under the `js` class, so they stay open here.
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

  # The row's `data-href` needs script; its name link must not.
  Scenario: A dashboard row reaches its check
    Given a check named "backup" with period 60
    When I visit the dashboard
    And I click the dashboard check link for "backup"
    Then I am on the check page

  # The filters are real GET forms that work without script.
  Scenario: The pings filter narrows the table
    Given a check named "backup" with period 60
    When I send a "success" ping
    And I send a "fail" ping
    And I reload the check page
    And I filter the pings by kind "fail"
    Then the pings table shows 1 rows
    And the pings kind filter shows "fail"

  # A GET submit replaces the whole query string; hidden fields carry the
  # other section's filter.
  Scenario: Filtering one section keeps the other section's filter
    Given a check named "backup" with period 60
    When I send a "success" ping
    And I send a "fail" ping
    And I reload the check page
    And I filter the notifications by event "down"
    And I filter the pings by kind "fail"
    # The row count proves a server round trip; the select values alone would
    # pass if merely set in the DOM.
    Then the pings table shows 1 rows
    And the pings kind filter shows "fail"
    And the notifications event filter shows "down"

  # With no `data-theme` (set only by `theme-init.js`), `prefers-color-scheme`
  # must pick the palette.
  Scenario: The OS colour scheme is honoured
    When my system prefers "light"
    And I visit the dashboard
    Then the page background is light
    When my system prefers "dark"
    And I visit the dashboard
    Then the page background is dark

  # Copy, LIVE and the theme toggle are pure `app.js`, so they are not drawn.
  Scenario: Controls that would do nothing are not drawn
    Given a check named "backup" with period 60
    Then the copy button is absent
    And the live tail toggle is absent
    And the theme toggle is absent

  # The age is rendered server-side; `app.js` only re-ticks it.
  Scenario: The scheduler heartbeat shows how long ago it ran
    When I open the admin dashboard
    Then the scheduler heartbeat shows an age

  # Per-kind fields are switched by `:has()` CSS rules, not script.
  Scenario: The check form shows only the selected schedule kind
    When I start creating a check
    Then the period field is visible
    And the cron field is hidden
    When I choose the "cron" schedule kind
    Then the cron field is visible
    And the period field is hidden

  # `data-confirm` is inert without script, so the server asks instead.
  Scenario: Deleting a check asks first, as a page
    Given a check named "backup" with period 60
    When I click the delete check button
    Then the confirmation page asks about deleting
    When I confirm the pending action
    Then I am on the project page for "Nightly jobs"
