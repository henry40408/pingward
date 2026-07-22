Feature: Static assets

  # `/favicon.svg` is served as image/svg+xml, so a browser parses it as XML,
  # not HTML. XML is strict where HTML is forgiving: every attribute needs a
  # value, and a comment may not contain a double hyphen. Either mistake is a
  # fatal parse error, and the failure is silent — no console error on the
  # page, just no icon in the tab. Firefox is strict here.
  #
  # This is asserted in the browser, over HTTP, against the bytes the server
  # actually sends, because that is the only thing that reproduces the
  # conditions: rendering the same SVG inlined into an HTML document (which is
  # how `npm run icons` rasterises it for the apple-touch-icon) goes through
  # the *HTML* parser and happily accepts markup that XML rejects.
  Scenario: The favicon is well-formed XML
    When I visit "/login"
    Then "/favicon.svg" is well-formed XML

  # The footer sits outside base.html's `show_nav` guard on purpose, so both
  # sides are asserted: the signed-out pages carry the version too, and those
  # are exactly the ones a broken instance gets reported from.
  Scenario: The build version is in the footer when signed out
    When I visit "/login"
    Then the footer shows the build version

  Scenario: The build version is in the footer when signed in
    Given an admin "admin" with password "password123" exists
    And I am signed in as "admin" with password "password123"
    Then the footer shows the build version
