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

  // Drive a ping against the check's ping URL. Supports all kinds; exitcode
  // needs opts.code; opts.method (default GET) and opts.body allow POST-body
  // capture tests. All valid pings return 200, so a non-ok status is a real
  // failure and still throws. Returns the Response.
  async ping(pingUrl, kind, opts = {}) {
    const { code, method = "GET", body } = opts;
    let target = pingUrl;
    if (kind === "fail") target = `${pingUrl}/fail`;
    else if (kind === "start") target = `${pingUrl}/start`;
    else if (kind === "log") target = `${pingUrl}/log`;
    else if (kind === "exitcode") target = `${pingUrl}/${code}`;
    const res = await fetch(target, {
      method,
      ...(body !== undefined ? { body } : {}),
    });
    if (!res.ok) {
      throw new Error(`ping (${kind}) failed: HTTP ${res.status}`);
    }
    return res;
  }

  // Fetch a ping URL and return only the HTTP status, without throwing on
  // error responses — used to assert the unknown-uuid 404 path.
  async pingStatus(url, { method = "GET" } = {}) {
    const res = await fetch(url, { method });
    return res.status;
  }
}
