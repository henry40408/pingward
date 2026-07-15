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
}
