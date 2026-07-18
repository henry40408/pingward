Feature: Check history pagination

  The check-detail page's "Recent pings" table shows only the newest 20 rows;
  keyset pagination lets the user page to older rows without an offset drift
  under concurrent inserts.

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"
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
