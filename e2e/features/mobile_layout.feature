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
    Then Environment rows do not wrap

  # The Check health and Notification health cards only render their tables
  # when there is failing data (down checks / failed deliveries), so this
  # seeds a check that goes down and a webhook channel bound to it whose
  # delivery fails (unreachable http://127.0.0.1:1/hook is the default target
  # for "I create a webhook channel"), populating all three conditional
  # tables at once: the down-checks table, the per-channel failure table, and
  # the recent-failures table. The project/check/channel names are long but
  # deliberately unbroken by spaces or hyphens — browsers treat those as line-
  # break opportunities, so a merely long name still wraps to fit a narrow
  # cell instead of forcing the table wider than the viewport.
  #
  # A regression here would be silent: with .cb's own overflow-x:auto, an
  # unwrapped wide table drags the rest of the card body sideways with it
  # (e.g. it would strand the "Recent failures" table below the down-checks
  # table off-screen), not just look like a scrollbar cosmetically appearing
  # in the wrong place.
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

  # The heartbeat caption is three flex items on one row: "30 runs ago", a
  # legend, and "now". Below ~640px they stop fitting, and because each is its
  # own flex item they wrapped independently and side by side — the legend's
  # second line landed beside "ago" rather than under its own first line. The
  # legend now takes a full-width row underneath instead.
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

  # A project group's header (.gh) is one non-wrapping flex row: name,
  # description, "N checks", a rule, and the "Manage →" link. Flex shrinks
  # every item with the default flex-shrink: 1, so a long description squeezed
  # the short labels too and broke "1 checks" into "1" / "checks" (and
  # "Manage →" into "Manage" / "→"). The description is the only item that may
  # give; it truncates with an ellipsis.
  Scenario: The dashboard group header labels stay on one line at phone width
    Given a project named "Nightly jobs"
    And I open the project edit form
    And I set the project description to "Backs up the primary database and every uploaded asset, then verifies the restore path end to end so a silent failure never goes unnoticed."
    And a check named "backup" with period 60
    When I view the site at 375px wide
    And I visit "/"
    Then the group header's count and manage link each stay on one line
