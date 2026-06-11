# Security Policy

## Reporting A Vulnerability

Do not publish exploit details in a public issue or public chat.

Preferred reporting channel: [GitHub Security Advisory](https://github.com/himawaril2dev/vocalsync-studio/security/advisories/new)

Fallback channel: email `himawaril2dev@gmail.com` with `VocalSync Security` in the subject.

Please include:

- VocalSync Studio version
- Operating system version
- A short impact summary
- Reproduction steps or a proof of concept
- Whether the issue can trigger code execution, unsafe file access, data exposure, or denial of service

## Supported Versions

Only the latest public portable release receives active security verification and fixes. Please upgrade to the newest release before reporting issues from an older build.

## Official Distribution

The official distribution channel is [GitHub Releases](https://github.com/himawaril2dev/vocalsync-studio/releases). Portable zip files from third-party websites may be modified or outdated.

## Release Verification

Verify portable zip files with SHA-256 before sharing or installing them:

```powershell
certutil -hashfile "VocalSync.Studio.Portable.x.y.z.zip" SHA256
```

Compare the result with the digest published on the same GitHub release page.
