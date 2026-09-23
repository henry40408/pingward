Feature: Mobile layout

  Phone-width (375px) layout. The header row (.bar .inner) must wrap, or its
  min-content width puts a horizontal scrollbar on every page; wide tables
  must scroll inside .tscroll rather than dragging their card body sideways.

  Background:
    Given an admin "admin" with password "correct horse battery" exists
    And I am signed in as "admin" with password "correct horse battery"

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
    Then Environment rows do not wrap

  # The health tables render only with failing data: a down check plus a
  # webhook bound to it whose delivery fails (the step's default target,
  # http://127.0.0.1:1/hook, is unreachable) populates all three. Names have
  # no spaces or hyphens, which would be break opportunities letting a narrow
  # cell wrap instead of forcing the table wider than the viewport.
  Scenario: The admin health tables scroll inside their cards at phone width
    Given a project named "NightlyMaintenanceAndReportingPipeline"
    And I remember the current project
    And I create a webhook channel named "ops_pager_primary_oncall_notification_channel"
    And a check named "backup_database_and_upload_to_remote_storage_snapshot_job" with period 60
    And I bind the channel "ops_pager_primary_oncall_notification_channel" to the check
    And I send a "fail" ping
    When I view the site at 375px wide
    And I visit "/admin"
    Then the admin health tables are shown
    And each admin health table scrolls inside its card, not the card around it

  Scenario: The check detail page has no horizontal scrollbar on a narrow viewport
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I view the site at 375px wide
    And I reload the check page
    Then the page has no horizontal scrollbar

  # The caption is three flex items ("older", legend, "now"); below ~640px
  # each would wrap independently and interleave, so the legend takes a
  # full-width row underneath.
  Scenario: The check page's heartbeat captions stack instead of interleaving at phone width
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I view the site at 375px wide
    And I reload the check page
    Then the heartbeat legend sits on its own row below the edge captions

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

  # The "no channel" chip stays visible below 640px (unlike .spark/.cwhen) but
  # moves to its own row: .nm has no ellipsis, so squeezing .cmeta would wrap
  # the name. Asserts the chip is rendered, the name is one line, and the chip
  # sits below it.
  Scenario: A check with no channel is flagged at phone width too
    Given a project named "Nightly jobs"
    And a check named "nightly-database-backup" with period 60
    When I view the site at 375px wide
    And I visit "/"
    Then the check row's name stays on one line beside the "no channel" chip
    And the page has no horizontal scrollbar

  # The group header (.gh) is one non-wrapping flex row; with the default
  # flex-shrink a long description would wrap "1 checks" and "Manage →". Only
  # the description may give, truncating with an ellipsis.
  Scenario: The dashboard group header labels stay on one line at phone width
    Given a project named "Nightly jobs"
    And I open the project edit form
    And I set the project description to "Backs up the primary database and every uploaded asset, then verifies the restore path end to end so a silent failure never goes unnoticed."
    And a check named "backup" with period 60
    When I view the site at 375px wide
    And I visit "/"
    Then the group header's count and manage link each stay on one line
