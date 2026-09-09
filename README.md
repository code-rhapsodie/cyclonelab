# cyclonelab

A generator and manipulation tool for [CycloneDX](https://cyclonedx.org/) 1.7 Software Bills of Materials (SBOMs).

`cyclonelab` is organized as a CLI with one subcommand per capability. It is not tied to any single ecosystem: the data
model (`src/cyclonedx`) implements the CycloneDX schema itself — components, licenses, hashes, tool metadata — and is
meant to grow new subcommands over time for whatever SBOM-generation or SBOM-editing task is needed next. The first
subcommand shipped, `generate-extension-sbom`, happens to target compiled PHP extension archives, but that's just
today's use case, not a constraint on the project.

## Installation

### Prebuilt binaries

Signed release binaries for Linux, Windows, and macOS (arm64 and x86_64) are published on
the [Releases page](https://github.com/code-rhapsodie/cyclonelab/releases) whenever a `v*` tag is pushed. Download the
archive matching your platform and put the `cyclonelab` (or `cyclonelab.exe`) binary on your `PATH`.

### From source

```sh
cargo install --path .
```

or, for development:

```sh
cargo build --release
./target/release/cyclonelab --help
```

## Usage

```sh
cyclonelab --help
```

```
Generator and manipulation tool for CycloneDX 1.7 SBOMs

Usage: cyclonelab <COMMAND>

Commands:
  generate-extension-sbom  Instantiates an SBOM template for each compiled PHP extension in an artifact folder
  help                     Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### `generate-extension-sbom`

Instantiates a CycloneDX SBOM template for each archive found in an artifact folder, substituting placeholders
(`{@version}`, `{@date_now}`, hashes, download URLs, ...) and registering `cyclonelab` as the sole `metadata.tools`
entry.

```sh
cyclonelab generate-extension-sbom \
  --version 2.3.0 \
  --php-version 8.3 \
  --template-path templates/template-sbom.cdx.json \
  --artifacts-dir artifacts \
  --repo owner/name
```

Run `cyclonelab generate-extension-sbom --help` for the full list of options and their defaults.

## Verifying release provenance

Every binary published on the [Releases page](https://github.com/code-rhapsodie/cyclonelab/releases) carries
a [SLSA build provenance attestation](https://slsa.dev/), generated in CI with `actions/attest-build-provenance` and
signed via Sigstore. It proves the binary was built by this repository's GitHub Actions workflow from a specific commit
and tag, not assembled or tampered with elsewhere.

After downloading a binary, verify it with the [GitHub CLI](https://cli.github.com/):

```sh
gh attestation verify ./cyclonelab-linux-x86_64 --repo code-rhapsodie/cyclonelab
```

A successful verification confirms the binary matches an attestation signed by the `code-rhapsodie/cyclonelab` release
workflow. Don't run a binary whose attestation fails to verify.

## License

Dual-licensed under your choice of:

- [European Union Public Licence 1.2](LICENSE-EUPL-1.2) (EUPL-1.2)
- [GNU Affero General Public License v3.0](LICENSE-AGPL-3.0) (AGPL-3.0)
