Feature: User management

  Admins manage the local user directory from /users: list, create,
  reset passwords, promote/demote, disable/enable, and delete accounts.
  Lockout guards protect the signed-in admin and the last enabled admin.

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"
    And I am on the users page

  Scenario: The seeded admin is listed as an admin
    Then the user "admin" is listed with role "admin"

  Scenario: Create a member
    When I add a user "member" with password "hunter2 correct"
    Then the user "member" is listed with role "member"

  Scenario: Create an admin
    When I add an admin user "boss" with password "hunter2 correct"
    Then the user "boss" is listed with role "admin"

  Scenario: Promote a member to admin
    Given a member "member" with password "hunter2 correct" exists
    When I toggle admin on "member"
    Then the user "member" is listed with role "admin"

  Scenario: Demote an admin to member
    Given an admin user "boss" with password "hunter2 correct" exists
    When I toggle admin on "boss"
    Then the user "boss" is listed with role "member"

  Scenario: Resetting a password lets the user sign in with the new one
    Given a member "member" with password "old pass one" exists
    When I reset "member"'s password to "new pass two"
    And I sign out
    And I am signed in as "member" with password "new pass two"
    Then I land on the dashboard signed in

  Scenario: A disabled user cannot sign in
    Given a member "member" with password "hunter2 correct" exists
    When I disable "member"
    Then the user "member" is marked disabled
    When I sign out
    And I sign in as "member" with password "hunter2 correct"
    Then the login page shows the error "account is disabled"

  Scenario: Re-enabling a disabled user restores sign-in
    Given a member "member" with password "hunter2 correct" exists
    And I disable "member"
    When I enable "member"
    Then the user "member" is not marked disabled
    When I sign out
    And I am signed in as "member" with password "hunter2 correct"
    Then I land on the dashboard signed in

  Scenario: Delete a user
    Given a member "member" with password "hunter2 correct" exists
    When I delete the user "member"
    Then the user "member" is not listed

  Scenario: The signed-in admin cannot delete their own account
    When I delete the user "admin"
    Then the user "admin" is listed with role "admin"

  Scenario: The signed-in admin cannot demote themselves
    When I toggle admin on "admin"
    Then the user "admin" is listed with role "admin"

  Scenario: The signed-in admin cannot disable themselves
    When I disable "admin"
    Then the user "admin" is not marked disabled

  Scenario: Self-management controls are inert on the signed-in admin's own row
    Then the demote control on my own row is inert
    And the disable control on my own row is inert
    And the delete control on my own row is inert
    And the password reset control on my own row is usable
