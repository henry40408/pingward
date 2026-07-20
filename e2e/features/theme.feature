Feature: Theme selection

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: The theme control cycles light, dark, then system
    Then the resolved theme is "light"
    When I click the theme toggle
    Then the stored theme preference is "light"
    And the resolved theme is "light"
    When I click the theme toggle
    Then the stored theme preference is "dark"
    And the resolved theme is "dark"
    When I click the theme toggle
    Then the stored theme preference is "system"

  # A global `a:hover` rule once outranked `.btn-primary`, so hovering the
  # dashboard's primary action swapped its label to a near-background colour
  # and the text all but vanished. Assert real contrast, in both themes.
  Scenario Outline: The primary action's label stays readable on hover
    Given I set the theme preference to "<theme>"
    When I hover the dashboard's primary action
    Then its label contrasts with its background

    Examples:
      | theme |
      | light |
      | dark  |

  Scenario: System mode follows the OS colour scheme
    Given I set the theme preference to "system"
    When the OS prefers dark
    Then the resolved theme is "dark"
    When the OS prefers light
    Then the resolved theme is "light"
