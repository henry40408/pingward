Feature: Authentication

  Scenario: A fresh instance sends visitors to setup
    When I visit "/"
    Then I am on the setup page

  Scenario: Creating the first admin signs you in
    Given I visit "/setup"
    When I create the admin "admin" with password "password123"
    Then I land on the dashboard signed in

  Scenario: Signing in with valid credentials
    Given an admin "admin" with password "password123" exists
    When I sign in as "admin" with password "password123"
    Then I land on the dashboard signed in

  Scenario: Signing in with a wrong password is rejected
    Given an admin "admin" with password "password123" exists
    When I sign in as "admin" with password "wrong-password"
    Then the login page shows the error "invalid username or password"

  Scenario: Signing out ends the session
    Given an admin "admin" with password "password123" exists
    And I am signed in as "admin" with password "password123"
    When I sign out
    Then I am on the login page
