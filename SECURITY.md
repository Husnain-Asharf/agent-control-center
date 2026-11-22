# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.2.x   | Yes       |
| 0.1.x   | No        |

## Reporting a vulnerability

Report vulnerabilities privately. Open the Security tab of this repository and choose "Report a vulnerability" (GitHub private vulnerability reporting). Do not open a public issue for a security problem.

Include the app version, your system (macOS version or Linux distribution), what you did, and what happened. We will reply as soon as we can and keep you updated until it is fixed.

## Threat model

Agent Control Center is local only. The daemon `accd` listens on a Unix socket in the data directory, with mode 0600, so only your user can connect. There are no TCP ports, no accounts and no telemetry.

Known limit: the socket has no authentication. Any process running as you can connect, including code that an agent runs inside a session. That code can do what the app does, such as approve its own requests. Authentication on the socket is planned for a later release. Until then, treat an agent that runs untrusted code as able to bypass the approvals inbox.

## What counts as a security issue

- A way to get an approval granted without the user.
- A saved rule that covers more than the approval showed.
- A protected path (credentials, the agent's own config) allowed automatically.
- A way for another user on the machine to reach the socket or the data directory.

Reports about the known limit above are welcome as ideas for the fix, but it is already tracked and is not treated as a new vulnerability.
