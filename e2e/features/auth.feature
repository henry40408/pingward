Feature: Authentication

  Scenario: A fresh instance sends visitors to setup
    When I visit "/"
    Then I am on the setup page
