# Security policy

## Reporting a vulnerability

**Please don't open a public issue for security findings.**

Use [GitHub Security Advisories](https://github.com/1337raspberry/ramus/security/advisories/new) — that's a private channel between you and the maintainers, with a built-in workflow for assigning a CVE if one's warranted.

I'll acknowledge receipt within **7 days** and aim to publish a fix within **90 days** of the report. If a fix is going to take longer (because it depends on an upstream library, or because the right fix is structural), I'll keep you in the loop.

You're welcome to publicly disclose after the 90-day window if I've gone silent. I'd rather you do that than sit on a real bug indefinitely.

## In scope

- Token / credential handling: the Plex auth token store, OAuth flow, Keychain / DPAPI / file-backed encryption.
- Plex client: HTTP client behaviour, redirect handling, response size limits, error-message redaction.
- FFI: libmpv runtime loading, drop ordering, Swift / Kotlin bridge handles.
- Cache database: SQL injection surfaces, FTS5 escaping, path traversal in image / audio cache paths.
- IPC: Tauri command surface, frontend → backend boundaries, native search bar contract.
- Mobile: iOS Keychain accessibility, Android cleartext config, ExoPlayer / MediaSession metadata.

## Out of scope

- Bugs in Plex Media Server itself — report those upstream to Plex.
- Bugs in libmpv, MPVKit, ExoPlayer, or other third-party runtime components — report upstream and let me know if ramus needs a workaround.
- Local-attacker scenarios where the threat model is "user already has shell on the device" (e.g. reading `~/Library/Application Support/`). That's a real concern but it's not what ramus can defend against; if you find a way to escalate that into a network-reachable issue, that's in scope.
- Issues that require the user to run a modified build of ramus themselves.

## Hall of thanks

If your report leads to a fix, you'll be credited in the release notes (and in the published advisory) unless you'd rather stay anonymous. Just say so in the report.
