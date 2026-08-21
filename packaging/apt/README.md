# Mantra APT repository

The Debian package workflow builds and validates `.deb` packages for `amd64` and `arm64`, then
attaches them to the matching GitHub Release. That makes direct installation possible, but an APT
repository additionally needs signed indexes and durable HTTPS hosting.

## Repository contract

The stable repository will use this layout:

```text
apt/
├── mantra-archive-keyring.pgp
├── pool/main/m/mantra/mantra_<version>-1_<architecture>.deb
└── dists/stable/
    ├── InRelease
    ├── Release
    ├── Release.gpg
    └── main/
        ├── binary-amd64/Packages.gz
        └── binary-arm64/Packages.gz
```

`Release` contains checksums for the package indexes. `InRelease` is the clear-signed form used by
APT, and `Release.gpg` is retained as a detached signature for compatible clients. Repository
publication must replace the complete generated tree atomically.

## Required protected configuration

Repository publication remains disabled until these values exist:

- `APT_GPG_PRIVATE_KEY`: exported private signing key stored as a GitHub Actions secret
- `APT_GPG_PASSPHRASE`: signing-key passphrase stored as a GitHub Actions secret
- final HTTPS repository URL, either a dedicated domain or a GitHub Pages repository

The public key will be exported as `mantra-archive-keyring.pgp`. The private key must never be
committed or exposed to pull-request workflows.

## Client configuration

Once hosting is selected, installation will follow this model:

```sh
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL <HTTPS_REPOSITORY_URL>/mantra-archive-keyring.pgp \
  | sudo tee /etc/apt/keyrings/mantra-archive-keyring.pgp >/dev/null

sudo tee /etc/apt/sources.list.d/mantra.sources >/dev/null <<'SOURCES'
Types: deb
URIs: <HTTPS_REPOSITORY_URL>/
Suites: stable
Components: main
Signed-By: /etc/apt/keyrings/mantra-archive-keyring.pgp
SOURCES

sudo apt update
sudo apt install mantra
```

The key is scoped with `Signed-By`; it must not be added to the system-wide legacy `apt-key`
keychain.
