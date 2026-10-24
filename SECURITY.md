# Security Policy

## Supported versions

Vine has not reached a stable release yet, so only the latest release and the `nightly` build get security fixes.

## Reporting a vulnerability

Please do not open a public issue for security problems.

Report them privately through [GitHub's vulnerability reporting](https://github.com/erilanetwork/Vine/security/advisories/new) instead. Include what you found, how to reproduce it and which version you tested. A proof of concept helps a lot, but is not required.

You should get a reply within a few days. Once a fix is out, the advisory is published with credit to you, unless you would rather stay anonymous.

## Scope

Things we especially care about:

- Bypassing online mode authentication or Vine forwarding signatures
- Reading or leaking `vine-forwarding.key` or other secrets
- Crashing the proxy or exhausting its memory from an unauthenticated connection
- Escaping the plugin sandbox or its memory and time limits

Running a backend that is reachable without the proxy, or with forwarding set to `none` on a public port, is a configuration issue rather than a vulnerability in Vine.
