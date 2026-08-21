# Mantra release automation

Mantra will use a single tagged GitHub release as the source of truth for every installation
channel. A package must never invent its own version or checksum. Downstream metadata is generated
from the immutable release manifest produced by the release workflow.

## Release flow

```text
vX.Y.Z tag
    │
    ├─ verify: fmt, clippy, tests, package metadata, version/tag match
    │
    ├─ build once per target
    │    ├─ macOS:  arm64, x86_64
    │    └─ Linux:  x86_64, arm64
    │
    ├─ publish GitHub Release
    │    ├─ mantra-<version>-<target>.tar.gz
    │    ├─ .deb packages
    │    ├─ SHA256SUMS + signature
    │    ├─ release-manifest.json
    │    └─ build provenance / attestations
    │
    └─ synchronize channels from release-manifest.json
         ├─ Homebrew tap
         ├─ AUR package
         ├─ signed APT repository
         └─ install.mantra.dev curl installer metadata
```

If any artifact build or verification fails, nothing downstream is published. Once the GitHub
release exists, each channel update is independently retryable and idempotent. Re-running a job for
the same version must either produce identical metadata or fail.

## Supported installation targets

Initial binaries:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`

Initial packages:

- Homebrew tap for Apple Silicon, Intel macOS, and supported Linux targets
- AUR `mantra-bin` package for `x86_64`; add source-built `mantra` after the build recipe stabilizes
- Debian/Ubuntu `.deb` packages for `amd64` and `arm64`
- A signed APT repository with a `stable` channel
- A curl installer that detects OS and architecture, verifies SHA-256, and installs only the
  matching GitHub Release artifact

Windows is outside the first packaging milestone and can be added after terminal behavior is tested
there.

## Version and artifact contract

The release workflow accepts only a SemVer tag in the form `vX.Y.Z`, and the tag version must equal
`Cargo.toml`. It produces `release-manifest.json` containing:

- version, Git commit, release URL, and publication timestamp
- artifact name, target, size, download URL, and SHA-256
- minimum supported OS/runtime information
- `.deb` package metadata
- signing identity and provenance references

Homebrew, AUR, APT, and the installer consume this manifest. Templates may differ by channel, but
version and checksum values may not be edited independently.

## Channel synchronization

### Homebrew

Maintain `Nabwinsaud/homebrew-tap` first so releases can synchronize without waiting for external
review. The release workflow updates the formula URLs and checksums, runs `brew audit`, installs the
formula on supported runners, and opens or merges a generated pull request. Once Mantra has stable
tagged releases and meets acceptance requirements, submit it separately to Homebrew Core.

Homebrew documents taps and formula updates in its
[tap guide](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap) and
[Formula Cookbook](https://docs.brew.sh/Formula-Cookbook).

### Arch Linux

Maintain `mantra-bin` in the AUR using a dedicated, revocable SSH key. A synchronization job updates
`pkgver`, source URLs, SHA-256 values, and `.SRCINFO`, then runs `makepkg`, `namcap`, and a clean
container smoke test. Keep publication behind a protected environment approval initially. After the
template has proven stable, version-only changes may publish automatically, while dependency,
license, install-layout, or template changes still require review.

This follows Arch's
[AUR submission guidance](https://wiki.archlinux.org/title/AUR_submission_guidelines), which allows
automation but requires active maintainer oversight.

### APT

Build `.deb` files from the same release binaries, test installation and removal in supported
Debian and Ubuntu containers, then add them to a GPG-signed repository. Publish the repository
atomically so clients never observe mismatched `Packages`, `Release`, or `InRelease` files. Keep the
signing key in a protected CI environment and publish a separate `mantra-archive-keyring` package to
support future key rotation.

The repository layout and signatures should follow Debian's
[repository format](https://wiki.debian.org/DebianRepository/Format) and
[reprepro setup guidance](https://wiki.debian.org/DebianRepository/SetupWithReprepro).

### Curl installer

Host a small, versioned shell script at `install.mantra.dev`. The script must use HTTPS, reject
unsupported OS/architecture combinations, download from the tagged GitHub Release, verify the
manifest SHA-256 before extraction, and install to a user-selected or documented prefix. Test it in
fresh macOS, Debian, Ubuntu, and Arch environments. The installer is a client of the release
manifest; it does not maintain a separate latest-version value.

## Rollout phases

### Phase 1 — release foundation

- Add CI for formatting, Clippy, tests, and a minimal terminal smoke test
- Add release metadata to `Cargo.toml` and enforce tag/version equality
- Build the four target archives with consistent names and contents
- Produce checksums, a JSON manifest, release notes, and build attestations
- Publish prereleases from `v0.x.y-rc.n` tags without updating stable channels

### Phase 2 — direct installer and Homebrew

- Implement and test the checksum-verifying curl installer
- Create the personal Homebrew tap and formula template
- Synchronize the tap from the release manifest
- Add post-publish installation tests for both channels

### Phase 3 — Linux packages

- Add reproducible `amd64` and `arm64` `.deb` packaging
- Create the signed APT repository and archive-keyring package
- Add `mantra-bin` to AUR with a protected publication environment
- Test upgrade and uninstall paths, not only fresh installation

### Phase 4 — stable synchronized releases

- Require every stable channel's validation before marking a GitHub Release generally available
- Add a dashboard/check that reports the version visible in every channel
- Add a scheduled drift check that opens an issue when any channel differs from the manifest
- Document rollback: yank or mark a release affected, restore prior package metadata, and never
  silently replace assets attached to an existing tag
- Pursue Homebrew Core and official distribution repositories only after project maturity and
  external acceptance requirements are met

## Release gates

A stable release is complete only when:

- tests, Clippy, formatting, and package smoke tests pass
- artifacts have checksums and provenance
- install, upgrade, and uninstall tests pass for every enabled channel
- the changelog and compatibility notes are present
- the channel versions match `release-manifest.json`
- secrets are scoped to protected publishing jobs and never exposed to pull-request workflows
- a failed downstream publish can be retried without rebuilding or replacing release artifacts
