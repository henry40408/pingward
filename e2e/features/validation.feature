Feature: Server-side form validation

  Server-side validation rejects input the client-side `required` attribute
  lets through (whitespace-only names) or does not constrain at all (the
  optional numeric override fields), re-rendering the form with an error.

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: A whitespace-only project name is rejected server-side
    Given I open the new project form
    When I fill the project name with "   "
    And I submit the project form
    Then the project form shows the error "name is required"

  Scenario: An invalid project scan interval is rejected and preserved
    Given I open the new project form
    When I fill the project name with "Nightly jobs"
    And I fill the project scan interval with "abc"
    And I submit the project form
    Then the project form shows the error "scan interval seconds must be a positive integer"
    And the project name field shows "Nightly jobs"

  Scenario: An invalid check max runtime is rejected
    Given a project named "Nightly jobs"
    And I open the new check form
    When I fill the check name with "backup"
    And I fill the check period with 60
    And I fill the check max runtime with "abc"
    And I submit the check form
    Then the check form shows the error "max runtime seconds must be a positive integer"
