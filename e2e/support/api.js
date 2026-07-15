// pingward has no open registration: the only bootstrap path is the one-time
// POST /setup (first admin), which is CSRF-exempt (pre-session). We POST the
// form and follow the redirect; a 2xx result means the admin was created.
// Used only against a fresh server, so the empty-field / already-set-up
// re-render branches never fire here.
export class ApiHelper {
  constructor(baseUrl) {
    this.baseUrl = baseUrl;
  }

  async bootstrapAdmin(username, password) {
    const body = new URLSearchParams({ username, password });
    const res = await fetch(`${this.baseUrl}/setup`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body,
    });
    if (!res.ok) {
      throw new Error(
        `bootstrapAdmin failed for "${username}": HTTP ${res.status}`
      );
    }
  }

  // Drive a ping against the exact URL the check page renders. The ping
  // endpoints are public and CSRF-exempt; a success ping marks the check up,
  // a fail ping marks it down (both synchronous within the request).
  async ping(pingUrl, kind) {
    const target = kind === "fail" ? `${pingUrl}/fail` : pingUrl;
    const res = await fetch(target);
    if (!res.ok) {
      throw new Error(`ping (${kind}) failed: HTTP ${res.status}`);
    }
  }
}
