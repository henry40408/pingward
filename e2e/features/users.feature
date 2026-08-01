Feature: User management

  Admins manage the local user directory from /users: list, create,
  reset passwords, promote/demote, disable/enable, and delete accounts.
  Lockout guards protect the signed-in admin and the last enabled admin.

  Background:
    Given an admin "admin" with password "correct horse battery" exists
    And I am signed in as "admin" with password "correct horse battery"
    And I unlock admin actions with my password "correct horse battery"
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
    Given a member "member" with password "old passphrase one" exists
    When I reset "member"'s password to "new passphrase two"
    And I sign out
    And I am signed in as "member" with password "new passphrase two"
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

  Scenario: Dismissing the delete confirmation leaves the user in place
    Given a member "member" with password "hunter2 correct" exists
    When I attempt to delete "member" but dismiss the confirmation
    Then the user "member" is listed with role "member"

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

  # The password length policy (`auth::validate_password`). Both admin-facing
  # surfaces are covered because the reset one used to answer a bad password
  # with a bare redirect back to /admin — indistinguishable from success.
  Scenario: A new user's password below the length floor is rejected
    When I try to add a user "carol" with password "short pass"
    Then the user form shows the error "Password must be at least 15 characters."
    And the user "carol" is not listed

  Scenario: A password reset below the length floor is rejected, not silently ignored
    Given a member "member" with password "hunter2 correct" exists
    When I try to reset "member"'s password to "short pass"
    Then the user form shows the error "Password must be at least 15 characters."
    And I sign in as "member" with password "hunter2 correct"
    Then I land on the dashboard signed in

  # The elevation gate (src/elevate.rs). The line is granting versus removing
  # access: handing out access that outlives this browser needs the password
  # again; taking access away must stay available to an operator who thinks
  # they are under attack.
  Scenario: Granting admin is refused until admin actions are unlocked
    Given a member "member" with password "hunter2 correct" exists
    And I lock admin actions
    When I try to grant admin to "member"
    Then I am sent to the confirmation page
    And the confirmation page explains the requirement
    And I am on the users page
    And the user "member" is listed with role "member"

  Scenario: Removing access stays available while admin actions are locked
    Given a member "member" with password "hunter2 correct" exists
    And I lock admin actions
    When I disable "member"
    Then the user "member" is marked disabled

