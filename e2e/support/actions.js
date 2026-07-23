// Shared UI actions used across step files.

// Drive the /login form: go to the login page, enter the credentials, submit.
// Callers assert the destination themselves — a successful login lands on "/",
// while an expected-failure login stays on /login with an error — so this
// helper deliberately stops at the click and makes no URL assertion.
//
// Switching accounts needs the explicit sign-out: /login bounces an
// already-authenticated visitor to "/" (which is what stops a forward-auth
// logout from showing a login form to someone the gateway has just signed back
// in), so the form is only reachable while signed out.
export async function signIn(page, serverUrl, username, password) {
  await page.goto(`${serverUrl}/login`);
  if (!page.url().endsWith("/login")) {
    await page.getByTestId("logout-button").click();
    await page.goto(`${serverUrl}/login`);
  }
  await page.getByTestId("username-input").fill(username);
  await page.getByTestId("password-input").fill(password);
  await page.getByTestId("login-submit").click();
}
