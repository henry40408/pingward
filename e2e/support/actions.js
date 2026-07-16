// Shared UI actions used across step files.

// Drive the /login form: go to the login page, enter the credentials, submit.
// Callers assert the destination themselves — a successful login lands on "/",
// while an expected-failure login stays on /login with an error — so this
// helper deliberately stops at the click and makes no URL assertion.
export async function signIn(page, serverUrl, username, password) {
  await page.goto(`${serverUrl}/login`);
  await page.getByTestId("username-input").fill(username);
  await page.getByTestId("password-input").fill(password);
  await page.getByTestId("login-submit").click();
}
