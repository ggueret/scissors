# Security Policy

## Supported versions

Only the latest released version of `scissors` receives security updates.

## Reporting a vulnerability

Please report security vulnerabilities **privately** using GitHub's
[private vulnerability reporting][advisory] for this repository. Do **not**
open a public issue for security reports.

[advisory]: https://github.com/ggueret/scissors/security/advisories/new

We aim to:

- acknowledge the report within 7 days,
- provide an initial assessment within 14 days,
- ship a fix or mitigation within 30 days when feasible.

For non-security bugs, please use the public [issue tracker][issues] instead.

[issues]: https://github.com/ggueret/scissors/issues

## Supply-chain posture

This project follows a hardened build and release posture; see the
[GitHub Security tab][security] for details:

- All GitHub Actions are pinned to full commit SHAs (enforced repo-wide).
- Releases are published to crates.io and PyPI via OIDC trusted publishing
  (no long-lived API tokens stored in CI).
- Release artifacts (wheels and binaries) carry [SLSA build provenance
  attestations][slsa], verifiable with `gh attestation verify`.
- Dependencies are audited on every CI run via `cargo-deny`
  (RustSec advisories, license allowlist, registry sources).
- `main` is protected: signed commits, linear history, no force-push,
  required PR + status checks.
- Branch protection bypass is reserved to the repository owner for ff-merges
  that preserve original signatures; all rules apply to external contributors.

[security]: https://github.com/ggueret/scissors/security
[slsa]: https://slsa.dev/spec/v1.0/levels#build-l2
