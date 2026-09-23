Feature: Check history pagination

  The check-detail page's "Recent pings" table shows only the newest 20 rows;
  keyset pagination lets the user page to older rows without an offset drift
  under concurrent inserts.

  Background:
    Given an admin "admin" with password "correct horse battery" exists
    And I am signed in as "admin" with password "correct horse battery"
    And a project named "Nightly jobs"
    And a check named "backup" with period 60

  Scenario: Paging through more than one page of pings
    When I send 25 "success" pings
    And I reload the check page
    Then the pings table shows 20 rows
    And the pings older link is enabled
    And the pings newer link is disabled
    When I click the pings older link
    Then the pings table shows 5 rows
    And the pings newer link is enabled
    And the pings older link is disabled
    When I click the pings newer link
    Then the pings table shows 20 rows
    And the pings older link is enabled

  Scenario: Filtering pings by kind refreshes the table in place
    When I send 3 "success" pings
    And I send 2 "fail" pings
    And I reload the check page
    Then the pings table shows 5 rows
    When I filter pings by kind "fail"
    Then the pings table shows 2 rows
    And the pings clear filter link is visible
    When I clear the pings filter
    Then the pings table shows 5 rows
    And the pings clear filter link is not visible

  Scenario: A datetime filter is retained after it is applied
    When I send 3 "success" pings
    And I reload the check page
    And I set the pings from date to "2020-01-01T00:00"
    And I apply the pings filter
    Then the pings from date is "2020-01-01T00:00"
    And the pings table shows 3 rows

  # CSS-only: the newest run stays pinned right (`justify-content: flex-end`)
  # and the overflow is clipped (`overflow: hidden`), not widening the page.
  # 60 runs overflow a phone-width strip.
  Scenario: The heartbeat strip clips its oldest runs instead of overflowing
    When I send 60 "success" pings
    And I view the site at 375px wide
    And I reload the check page
    Then the newest heartbeat bar is flush with the strip's right edge
    And the oldest heartbeat bars are clipped off the left
    And the page has no horizontal scrollbar

  # Pairs with no_js.feature: with script the panel must start collapsed, so
  # leaving every panel open cannot pass both.
  Scenario: The captured output stays collapsed until the row is clicked
    When I send a failing ping with output "boom: disk full"
    And I reload the check page
    Then the captured output "boom: disk full" is hidden
    When I expand the first ping row
    Then the captured output "boom: disk full" is visible
