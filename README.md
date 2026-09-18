# cyclonelab

A generator and manipulation tool for [CycloneDX](https://cyclonedx.org/) 1.7 Software Bills of Materials (SBOMs).

`cyclonelab` is organized as a CLI with one subcommand per capability. It is not tied to any single ecosystem: the data
model (`src/cyclonedx`) implements the CycloneDX schema itself — components, licenses, hashes, tool metadata — and is
meant to grow new subcommands over time for whatever SBOM-generation or SBOM-editing task is needed next.

To generate a transform recipe with LLM, a prompt example is provided into [llms_prompt_example.md](llms_prompt_example.md).

You can provide the file [llms.md](llms.md) to your favorite LLM to explain how to cyclonelab work.

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
  validate   Checks that a file is valid JSON and conforms to the CycloneDX schema
  transform  Applies a declarative transformation recipe to a CycloneDX SBOM
  suggest    Suggests useful component fields missing from a CycloneDX SBOM
  help       Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

Run `cyclonelab <COMMAND> --help` for each subcommand's full list of options; see `llms.md` for detailed usage and the
`transform` YAML recipe format.

## Scope: building and enriching a single SBOM

`cyclonelab` focuses on building and enriching a single SBOM: generating one, validating it, and applying
transformations to it. It does not merge multiple SBOMs into one.

If your release pipeline produces several SBOMs (for example one per image, service, or ecosystem) and you need to
combine them into a single consolidated SBOM, use the [CycloneDX CLI](https://github.com/CycloneDX/cyclonedx-cli)'s
`merge` command as a separate step in your CI, before or after running `cyclonelab` on the result.

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

If you want to verify with [cosign](https://github.com/sigstore/cosign), use these options: `--certificate-oidc-issuer="https://token.actions.githubusercontent.com" --certificate-identity-regexp="^https://github.com/code-rhapsodie/cyclonelab/"`

## License

Dual-licensed under your choice of:

- [European Union Public Licence 1.2](LICENSE-EUPL-1.2) (EUPL-1.2)
- [GNU Affero General Public License v3.0](LICENSE-AGPL-3.0) (AGPL-3.0)
