Feature: Static assets

  # Served as image/svg+xml, so it is parsed as strict XML (valueless
  # attributes or `--` in a comment are fatal); the failure is silent, just no
  # tab icon. Checked against the bytes the server actually sends.
  Scenario: The favicon is well-formed XML
    When I visit "/login"
    Then "/favicon.svg" is well-formed XML

  # The footer is outside base.html's `show_nav` guard, so signed-out pages
  # carry the version too; both sides are asserted.
  Scenario: The build version is in the footer when signed out
    When I visit "/login"
    Then the footer shows the build version

  Scenario: The build version is in the footer when signed in
    Given an admin "admin" with password "password123 stapler" exists
    And I am signed in as "admin" with password "password123 stapler"
    Then the footer shows the build version
