Feature: Admin cross-user management

  Admins reach every user's data through the audited /admin/* route surface:
  a site-wide dashboard, a projects list annotated with owners, and full
  view/edit/delete/pause/resume/ack/regenerate/channel control over projects
  and checks they do not own. The admin entity pages reuse the owner templates
  (same data-testid selectors) with /admin-prefixed forms.

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"
    And a non-admin user "member" with password "hunter2 correct" exists
    And "member" with password "hunter2 correct" owns a project "Member jobs" with a check "member-backup" period 60
    And I am signed in as "admin" with password "correct horse"

  Scenario: The admin dashboard loads with site-wide stats
    When I open the admin dashboard
    Then the admin dashboard is shown

  # .subhead ("Recent failures", the Environment groups, "Add user") only set
  # spacing, so it inherited the global h2 and rendered at 21px/700 inside a
  # card whose own heading is 13px — the section shouting over its container.
  Scenario: A subheading inside a card does not outweigh the card's own heading
    When I open the admin dashboard
    Then no card subheading renders larger than its card heading

  @smtp-env
  Scenario: The admin Environment card shows SMTP config as configured without leaking the secret
    When I open the admin dashboard
    Then the Environment card shows the SMTP password as configured
    And the page does not contain the SMTP secret

  Scenario: The admin projects list shows every user's project with its owner
    When I open the admin projects list
    Then the admin projects list shows "Member jobs" owned by "member"

  Scenario: The admin can view another user's check
    When I open the member's check in the admin area
    Then I am viewing the check "member-backup"
    And the check status is "new"
    And the ping URL is shown

  Scenario: The admin can pause and resume another user's check
    Given I open the member's check in the admin area
    When I pause the check
    Then the check status is "paused"
    When I resume the check
    Then the check status is not "paused"

  Scenario: The admin can acknowledge another user's down check
    Given I open the member's check in the admin area
    And I send a "fail" ping
    And I reload the check page
    When I acknowledge the check
    Then the acknowledge control is gone

  Scenario: The admin can regenerate another user's ping URL
    Given I open the member's check in the admin area
    When I regenerate the ping URL
    Then the ping URL is different from before

  Scenario: The admin can rename another user's project
    Given I open the member's project in the admin area
    When I rename the project to "Renamed by admin"
    Then I am on the admin project page for "Renamed by admin"

  Scenario: The admin can add a check to another user's project
    Given I open the member's project in the admin area
    When I create a check named "admin-added" with period 120
    Then I am viewing the check "admin-added"

  Scenario: The admin can create a notification channel on another user's project
    Given I open the member's project in the admin area
    When I add a webhook channel named "ops hook"
    Then the channel "ops hook" is listed on the project

  Scenario: The admin can delete another user's check
    Given I open the member's check in the admin area
    When I delete the check
    Then the project has no checks

  Scenario: The admin can delete another user's project
    Given I open the member's project in the admin area
    When I delete the member's project
    Then the admin projects list has no projects
