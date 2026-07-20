Feature: Authorization and security boundaries

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: An admin sees the Admin nav link
    Then the "Admin" nav link is visible

  Scenario: A non-admin does not see the Admin nav link
    Given a non-admin user "member" with password "hunter2 correct" exists
    And I sign out
    And I am signed in as "member" with password "hunter2 correct"
    Then the "Admin" nav link is not visible

  Scenario: A non-admin is forbidden from the admin area
    Given a non-admin user "member" with password "hunter2 correct" exists
    And I sign out
    And I am signed in as "member" with password "hunter2 correct"
    When I navigate to "/admin"
    Then the response status is 403

  Scenario: A non-admin cannot read another user's project
    Given a non-admin user "member" with password "hunter2 correct" exists
    And I create a project named "Secret jobs"
    And I remember the current project
    And the owner can read the remembered project
    When I revisit it as "member" with password "hunter2 correct"
    Then the response status is 404

  Scenario: A logged-out visitor is redirected to login
    Given I sign out
    When I navigate to "/"
    Then I am on the login page

  Scenario: A POST without a CSRF token is rejected
    Given the "Admin" nav link is visible
    When I POST to "/projects" without a CSRF token
    Then the response status is 403
