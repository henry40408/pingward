Feature: Authentication

  Scenario: A fresh instance sends visitors to setup
    When I visit "/"
    Then I am on the setup page

  Scenario: Creating the first admin signs you in
    Given I visit "/setup"
    When I create the admin "admin" with password "password123"
    Then I land on the dashboard signed in
