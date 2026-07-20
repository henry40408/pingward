Feature: Mobile layout

  The sticky header (.bar .inner) lays out brand, nav links and controls in a
  single non-wrapping flex row whose intrinsic width is fixed regardless of
  viewport. Nothing else on these pages causes horizontal overflow, so a
  regression here would silently reintroduce a horizontal scrollbar on every
  phone-width viewport across the app.

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario Outline: <page> has no horizontal scrollbar on a narrow viewport
    When I view the site at 375px wide
    And I visit "<page>"
    Then the page has no horizontal scrollbar

    Examples:
      | page             |
      | /                |
      | /admin           |

  Scenario: The admin Add user form does not scroll with the users table
    When I view the site at 375px wide
    And I visit "/admin"
    Then only the users table scrolls sideways, not the card around it

  Scenario: The admin Environment rows stay short at phone width
    When I view the site at 375px wide
    And I visit "/admin"
    Then no Environment row is taller than 72px

  Scenario: The check detail page has no horizontal scrollbar on a narrow viewport
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I view the site at 375px wide
    And I reload the check page
    Then the page has no horizontal scrollbar

  Scenario: The project page's check rows stay on one line at phone width
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I view the site at 375px wide
    And I open the project from the breadcrumb
    Then the check row's status dot sits next to the name
    And the check row is a single line

  Scenario: The dashboard's check rows stay on one line at phone width
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I view the site at 375px wide
    And I visit "/"
    Then the check row's status dot sits next to the name
    And the check row is a single line
