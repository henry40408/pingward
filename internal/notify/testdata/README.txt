requests.json: requests received by a plain HTTP sink for the webhook (/hook), slack (/slack)
and ntfy (/pw) notifiers, driven by the Rust v0.12.0 image over host.docker.internal.
Events: test, down (overdue / failed / overrun), up, reminder (first one per sink only).
Timestamps and check ids vary. NOT captured: telegram and pushover (fixed hosts) and email
(.eml needs a mailpit container, second container).
