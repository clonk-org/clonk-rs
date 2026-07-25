# Security policy

## Supported code

Clonk Rust does not yet publish stable release branches. Security fixes target
the current `main` branch; older commits and local forks are not maintained
separately.

## Reporting a vulnerability

Do not disclose a suspected vulnerability in a public issue, discussion, pull
request, replay, log, or test fixture.

Use GitHub's private vulnerability reporting for this repository:

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability**.
3. Describe the affected revision, impact, reproduction steps, and any known
   mitigation.

If **Report a vulnerability** is unavailable, private reporting has not yet
been enabled by the repository owner. Do not publish sensitive details; ask
the owner through an existing private channel to enable GitHub private
vulnerability reporting.

Please include only the data needed to reproduce the issue and remove player
identities, credentials, tokens, and unrelated network traffic. The project
does not promise a fixed response or remediation timeline, but maintainers
should acknowledge reports and coordinate disclosure through the private
advisory.

This policy covers the Rust engine and client, networking and resource
handling, bundled scripts and tooling, and the repository's own CI
configuration. Vulnerabilities in an upstream dependency should also be
reported to that dependency's maintainers.
