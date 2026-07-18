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
    And the pings older link is visible
    And the pings newer link is not visible
    When I click the pings older link
    Then the pings table shows 5 rows
    And the pings newer link is visible
    And the pings older link is not visible
    When I click the pings newer link
    Then the pings table shows 20 rows
    And the pings older link is visible
