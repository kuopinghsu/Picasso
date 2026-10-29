# Releasing Picasso

This guide describes how to prepare, publish and verify a Picasso release. The
[CI workflow](../.github/workflows/ci.yml) builds the tagged commit and publishes its
packages on GitHub Releases after the required jobs succeed.

A release tag must match the version in [Cargo.toml](../Cargo.toml). For example,
`v0.9.5` requires version `0.9.5` and a nonempty section for that version in
[CHANGELOG.md](../CHANGELOG.md). Replace `Unreleased` with the release date before
pushing the tag. Tags containing a hyphen, such as `v1.0.0-beta.1`, are published
as pre-releases.

Release preparation and verification are separate tasks. Successful CI builds do
not replace the manual application checks in [TESTING.md](TESTING.md).

## Before you start

- Use an up-to-date checkout of `main`. Finish or save unrelated changes before
  preparing a release.
- Install the Rust version required by Cargo.toml and the platform build tools
  listed in the [README](../README.md#development).
- Install cargo-deny with `cargo install cargo-deny --locked` for local dependency checks.
- Confirm that you can push the release commit and tag to the repository.
- If the macOS release must be notarized, configure the signing secrets described
  under [macOS signing](#macos-signing) before starting the workflow.

The examples below use `0.9.5`. Replace it with the version you intend to release.
Use an unused version if its tag has already been pushed.

## Prepare the release commit

1. Update `version` in Cargo.toml, then run `cargo build` to update Cargo.lock.
   Review both files before committing.
2. Prepare the matching changelog section. Describe changes that users will notice
   and use a heading such as `## [0.9.5] - YYYY-MM-DD`, with the actual release date.
   Update its comparison link at the bottom of the file:

   ```text
   [0.9.5]: https://github.com/kuopinghsu/picasso/compare/v0.9.4...v0.9.5
   ```

3. Run the local checks from the repository root:

   ```sh
   make ci
   bash scripts/release-notes.sh --release 0.9.5
   git diff --check
   ```

   `make ci` checks documentation links, commands and settings; dependency
   advisories, licenses and sources; release-mode tests; and benchmark limits.
   The benchmark check uses 20,000 generated photos. See
   [PERFORMANCE.md](PERFORMANCE.md) for its methodology and limits.

   The release-notes command checks that the section exists, is nonempty and no
   longer says `Unreleased`. It prints the changelog portion of the notes.
   `make release-notes` previews the current version's section without requiring
   a release date. Neither command includes the download instructions that CI adds.

4. Review and commit the version files, changelog and any intended release changes.
   Push the commit to `main` and wait for its CI run to pass.
5. Complete the manual checks in
   [Before a release, by hand](TESTING.md#before-a-release-by-hand) on the supported
   platforms. Record the systems, processor architectures and results in the
   release pull request or release notes. Fix any problems before tagging; a new
   commit requires a new CI run.

## Tag the verified commit

Confirm that the working tree has no pending release changes and that `HEAD` is
the exact commit whose CI run and manual checks passed:

```sh
git status --short
git log -1 --oneline
git rev-parse HEAD
```

Compare the printed commit with the successful CI run. Do not tag a newer,
unchecked commit. Then create and push an annotated tag:

```sh
git tag -a v0.9.5 -m "Picasso 0.9.5"
git push origin v0.9.5
```

Pushing the tag starts another CI run for that commit. Runs targeting a version
tag can publish a release, including manual workflow runs targeting that tag.
Pushes to `main`, pull requests and manual runs targeting branches produce build
artifacts without publishing a release. Non-tag artifact names include the short
source commit after the version.

## What CI checks and publishes

The release job waits for documentation and changelog checks, dependency checks,
Windows builds, Linux builds, AppImage builds and startup tests, the macOS build,
and PDF guide generation.

- **Windows:** release-mode tests and packages on native x64 and Arm64 runners.
- **Linux:** release-mode tests, benchmark limits, and Debian package installation,
  version, startup and removal checks on x64 and Arm64.
- **AppImage:** builds for both architectures and startup tests on AlmaLinux 8
  and Ubuntu 24.04.
- **macOS:** tests on the runner and a universal application containing Apple
  silicon and Intel executables. This does not replace testing on both kinds of Mac.
- **Documentation:** English and Traditional Chinese PDF guides. Missing font
  coverage causes the guide build to fail.

Documentation and version checks run before the build and guide jobs, so invalid
release metadata stops expensive work early. Dependency auditing runs in parallel
and remains required for publication. Rust jobs retain dependency caches after a
failure; a cache hit does not skip tests. macOS tests use the explicit native target
also used by the universal build, allowing Cargo to reuse compiled dependencies.
Already-compressed Linux/macOS packages and PDF artifacts upload without another
compression pass; Windows artifacts retain compression for the bare executable.

Linux and AppImage builds intentionally remain separate: the Ubuntu packages target
glibc 2.35, while AppImage is built on AlmaLinux 8 for glibc 2.28 compatibility.
All native architecture tests and the AppImage startup matrix remain enabled for
pull requests, main-branch builds and version tags. These changes reduce avoidable
work; actual elapsed-time savings depend on cache hits and runner availability.

Before publication, CI checks that a GitHub release for the tag does not already
exist and that the required application packages and guides are present and
nonempty. It then downloads the locked rawler source archive, verifies its
checksum against Cargo.lock, and generates checksums for the release files.

The publication check does not permit replacing an existing release. It is not
an automatic rollback mechanism if publication or asset upload fails.

### Expected release files

For Picasso `0.9.5`, the release contains these 16 files:

```text
picasso-0.9.5-windows-x64.exe
picasso-0.9.5-windows-x64.zip
picasso-0.9.5-windows-arm64.exe
picasso-0.9.5-windows-arm64.zip
picasso-0.9.5-linux-x64.tar.gz
picasso-0.9.5-linux-x64.deb
picasso-0.9.5-linux-x64.AppImage
picasso-0.9.5-linux-arm64.tar.gz
picasso-0.9.5-linux-arm64.deb
picasso-0.9.5-linux-arm64.AppImage
picasso-0.9.5-macos-universal.dmg
picasso-0.9.5-macos-universal.zip
picasso-0.9.5-user-guide.pdf
picasso-0.9.5-user-guide-zh-TW.pdf
rawler-<rawler-version>-source.tar.gz
SHA256SUMS.txt
```

The rawler filename uses **rawler's version from Cargo.lock**, not Picasso's
version. GitHub may also display automatically generated source archives; these
are separate from the files uploaded by the release workflow.

## Verify the published release

- [ ] The tag, Cargo.toml version and dated changelog heading agree.
- [ ] All required jobs passed in the tag's CI run.
- [ ] All twelve application packages, both PDF guides, the rawler source archive
  and SHA256SUMS.txt are available with the expected names.
- [ ] Downloaded files match their published checksums.
- [ ] The release notes identify the source commit. **About Picasso** and
  `picasso --version` in the downloaded builds identify the same commit.
- [ ] The macOS signing and notarization statement matches the package's behavior
  on first launch.
- [ ] Manual application checks are complete and their results are recorded.
- [ ] Both PDF guides open correctly, including screenshots and Chinese text.

To verify a single download, calculate its SHA-256 checksum and compare the full
value and filename with its entry in SHA256SUMS.txt. For example, on macOS:

```sh
shasum -a 256 picasso-0.9.5-macos-universal.dmg
```

On Windows PowerShell:

```powershell
Get-FileHash .\picasso-0.9.5-windows-x64.exe -Algorithm SHA256
```

On Linux, when SHA256SUMS.txt and all listed files are in the current folder:

```sh
sha256sum --check SHA256SUMS.txt
```

Every listed file should report `OK`. A mismatch requires investigation before
using or distributing that download.

## Start the next development version

After verifying the release, add a section such as `## [0.9.6] - Unreleased` at
the top of CHANGELOG.md and a comparison link at the bottom:

```text
[0.9.6]: https://github.com/kuopinghsu/picasso/compare/v0.9.5...HEAD
```

Record subsequent changes in that section. When changing the package version,
update Cargo.toml and Cargo.lock together and keep a nonempty matching changelog
section so the main-branch checks continue to pass.

## Recover from a failed or incorrect release

Keep pushed tags and published packages unchanged. Do not delete or reuse a pushed
tag, force-push it to another commit, or replace published assets. This keeps a
version associated with the same source and downloaded files.

### A workflow failed without requiring a source change

Inspect the failed job and check whether a GitHub release or any assets already
exist for the tag. A failed workflow does not necessarily mean nothing was published.

If publication has not started and the failure was temporary, such as a network
or runner outage, rerun the failed jobs for the **same tag and commit**. Review
all required job results before accepting the release. If a release already
exists, the publication job refuses to run again; use a new patch version for
a correction or incomplete publication.

### A source, packaging or documentation change is required

1. Fix the problem on `main`, preserving the original tag and any published files.
2. Select a new, unused patch version, such as `0.9.6`. Update Cargo.toml and
   Cargo.lock and add a dated, nonempty changelog section describing the correction.
3. Repeat the local checks, main-branch CI run and relevant manual checks.
4. Tag the newly verified commit with the new version and push that tag.
5. Verify the new release and record which earlier version it corrects.

If the replacement also fails and requires changes, repeat with another unused
version. Do not bypass a failed check to publish the original version.

## macOS signing

Without signing credentials, CI creates an ad-hoc signed application. It is not
notarized, so users may need **Open Anyway** in **System Settings → Privacy & Security**
on first launch. With signing credentials, the workflow signs with a Developer ID,
submits the app and DMG for notarization, and staples the results.

Configure these repository Actions secrets before the release run:

- `MACOS_CERTIFICATE`: the Developer ID Application certificate and private key,
  exported as a `.p12` file and encoded with `base64 -i certificate.p12`.
- `MACOS_CERTIFICATE_PASSWORD`: the password used to export that `.p12` file.
- `APPLE_ID`: the Apple Account used for notarization.
- `APPLE_TEAM_ID`: the associated developer team ID.
- `APPLE_APP_PASSWORD`: an app-specific password for the Apple Account.

The workflow selects notarization when the certificate, Apple ID, team ID and
app-specific password are nonempty. It passes the certificate password to the
certificate import step. Partial configuration can therefore select the ad-hoc
path or fail during signing; configure and verify the complete set when a
notarized release is required.

Once the notarization path is selected, a signing or notarization failure fails
the macOS job. The workflow does not fall back to publishing an ad-hoc build.
The generated release notes report the signing outcome.

## Dependencies and license files

`make deny` runs cargo-deny with [deny.toml](../deny.toml). The policy checks
security advisories, yanked crates, allowed licenses, dependency sources and
wildcard requirements for the configured release targets; development dependencies
are excluded. Fix dependency issues before tagging. After changing a dependency,
update Cargo.lock and run the relevant tests and checks.

Review a new license before changing the allow list. Keep crate-specific exceptions
limited to the named crate and document the reason in deny.toml.

`make notices` generates THIRD-PARTY-NOTICES.txt for the host platform. CI runs the
[notice generator](../scripts/third-party-notices.py) for each packaged target and
includes the result in every application package except the standalone Windows
`.exe`. In the macOS application, the file is in `Contents/Resources`.
The AppImage also includes license files for its bundled system libraries.

The generator uses license texts from the crates. If a crate has no license text,
it uses the corresponding standard text in [build/licenses](../build/licenses)
and fails if that text is unavailable.

The release workflow also attaches the exact rawler crate source archive used by
the lockfile as `rawler-<rawler-version>-source.tar.gz`. Its checksum must match
Cargo.lock. Keep this archive and the generated notices in the release, including
when changing the RAW decoder version; the notices identify the source locations
and how Picasso can be rebuilt with a modified rawler.
