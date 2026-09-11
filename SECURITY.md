# Security Policy

## Reporting a vulnerability

Please report security issues privately using one of these channels:

- **GitHub**: open the [private vulnerability report](https://github.com/safonas/ocid/security/advisories/new) form
- **Email**: security@ocid.dev

We aim to acknowledge reports within 48 hours and coordinate a fix and disclosure.

## Supported versions

Only the latest release receives security fixes.

## Scanning

Container images and dependencies are scanned for vulnerabilities in CI
([`.github/workflows/security.yml`](.github/workflows/security.yml)) using:

- **Trivy** — container image and filesystem CVE / secret / misconfiguration scanning
- **OpenSSF Scorecard** — repository security posture

To scan locally:

```sh
trivy image localhost/ocid:dev     # scan the image built by `just image`
trivy fs .                         # scan the working tree
```

Report a vulnerability found in a dependency via the private channel above.
