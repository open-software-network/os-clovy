# Security policy

## Supported versions

Security fixes target `main`. There are no published releases; build from the
latest `main`.

## Reporting a vulnerability

Please do not file public issues for suspected vulnerabilities.

Use GitHub private vulnerability reporting when it is enabled for this
repository. If it is not available, contact the repository maintainer
privately.

Include the affected component, reproduction steps, impact, and any relevant
logs or proof of concept. We will acknowledge the report, keep the discussion
private while we investigate, and coordinate disclosure timing with you.

## Scope

In scope:

- Clovy desktop app local storage, permissions, and credential handling.
- Direct provider credential isolation, request validation, bounded media
  downloads, and CLI/MCP turn authorization.
- GitHub Actions workflows.

Out of scope:

- Social engineering.
- Denial of service without a clear security impact.
- Issues that require physical access to an already compromised device.
