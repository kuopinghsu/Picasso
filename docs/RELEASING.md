# Releasing

A release is made by pushing a tag like `v0.9.5`. CI builds and tests the tagged commit on every platform, packages it, and publishes the packages on a GitHub release with the version's section of `CHANGELOG.md` as the notes. CI refuses to publish when the tag doesn't match the version in `Cargo.toml`, when `CHANGELOG.md` has no dated section for it, when a package is missing, or when the version is already published.

## Steps

1. Change `version` in `Cargo.toml`, then run `cargo build` so that `Cargo.lock` follows.
2. In `CHANGELOG.md`, change the version's heading from `## [0.9.5] - Unreleased` to the release date, such as `## [0.9.5] - 2026-11-02`, and read it through. At the bottom, point the version's link at the tag: `[0.9.5]: https://github.com/kuopinghsu/picasso/compare/v0.9.4...v0.9.5`.
3. Run the checks CI runs, so that a failure shows up before the tag:

   ```bash
   make ci            # docs, deny, tests, and bench-check, as below
   make release-notes # the notes as they will be published
   ```

   `make ci` runs `make check-docs` (links, make targets, and settings named in the docs), `make deny` (security advisories, licenses, and crate sources), `make test` (unit and integration tests), and `make bench-check` (20,000 photos within the limits in docs/PERFORMANCE.md).

4. Commit, push to `main`, and wait for CI to pass on every platform. The tag builds the commit it points to, so a commit that fails CI makes a release that fails.
5. Tag that commit and push the tag:

   ```bash
   git tag -a v0.9.5 -m "Picasso 0.9.5"
   git push origin v0.9.5
   ```

6. Go through the checklist below with the published packages.
7. Start the next version: add a `## [0.9.6] - Unreleased` section at the top of `CHANGELOG.md` and a `[0.9.6]: https://github.com/kuopinghsu/picasso/compare/v0.9.5...HEAD` link at the bottom, and note changes there as they are made.

## Checklist

- [ ] The version in `Cargo.toml`, the tag, and the heading in `CHANGELOG.md` agree, and the changelog says everything a user will notice.
- [ ] CI passed on Windows and Linux for x64 and Arm64 and on macOS, including the .deb install tests, the AppImage tests on Ubuntu and AlmaLinux, the performance checks, and `cargo deny`.
- [ ] The release has the twelve packages, each named `picasso-<version>-<platform>`: `windows-x64.exe`, `windows-x64.zip`, `linux-x64.tar.gz`, `linux-x64.deb`, and `linux-x64.AppImage`, the same five with `arm64` in place of `x64`, and `macos-universal.dmg` and `macos-universal.zip`, plus the User Guide as `picasso-<version>-user-guide.pdf` and `picasso-<version>-user-guide-zh-TW.pdf`, `rawler-<version>-source.tar.gz`, and `SHA256SUMS.txt`.
- [ ] A downloaded package matches its line in `SHA256SUMS.txt` (`sha256sum --check --ignore-missing SHA256SUMS.txt`, or `shasum -a 256 --check` on a Mac).
- [ ] The notes name the commit, and **About** and `picasso --version` in the packages name the same one.
- [ ] The notes say whether the Mac app is notarized, and opening it behaves as they say.
- [ ] The checks in "Before a release, by hand" in [`TESTING.md`](TESTING.md) were done on each platform, and what was tried, on which system, is written in the release notes or the pull request.

## A published version never changes

If a release fails before it is published, for example because a build broke, fix the problem on `main`, then delete the tag and tag the new commit:

```bash
git tag -d v0.9.5
git push origin :refs/tags/v0.9.5
git tag -a v0.9.5 -m "Picasso 0.9.5"
git push origin v0.9.5
```

Once a release is published, its tag and packages stay as they are, because people may have downloaded them and checked them against `SHA256SUMS.txt`. A problem found afterwards is fixed in the next patch version, such as 0.9.6. CI checks that the version isn't published yet, so moving a published tag doesn't replace its packages.

## Signing and notarizing the Mac app

Without these secrets, CI gives the Mac app an ad-hoc signature, and users have to allow it with **Open Anyway** in **System Settings → Privacy & Security** the first time. With them, it signs the app with a Developer ID and has Apple notarize the app and the `.dmg`, so they open like any downloaded app. Set them in the repository's **Settings → Secrets and variables → Actions**:

| Secret | What it is |
| --- | --- |
| `MACOS_CERTIFICATE` | The Developer ID Application certificate with its private key, exported from Keychain Access as a `.p12` and encoded with `base64 -i certificate.p12` |
| `MACOS_CERTIFICATE_PASSWORD` | The password chosen when exporting the `.p12` |
| `APPLE_ID` | The Apple Account of the developer team |
| `APPLE_TEAM_ID` | The team's ID, shown under Membership on developer.apple.com |
| `APPLE_APP_PASSWORD` | An app-specific password for that account, made at account.apple.com under Sign-In and Security |

A Developer ID certificate needs a paid Apple Developer Program membership. Make it in Xcode under **Settings → Accounts → Manage Certificates**, or at developer.apple.com under Certificates. When the secrets are set, a failed notarization fails the build instead of publishing an app that isn't notarized.

## Dependencies and licenses

`make deny` runs `cargo deny` with `deny.toml`: it fails on a security advisory or a yanked crate in the dependencies, on a license not in its list, and on crates from anywhere but crates.io. To fix an advisory, update the crate with `cargo update -p <crate>` and run the tests. A new license is added to `deny.toml` only after reading it; a license that asks for more than a notice, like rawler's LGPL-2.1, is listed as an exception for that crate alone, with the reason.

`make notices` writes `THIRD-PARTY-NOTICES.txt`, the license of every crate built into Picasso, which CI puts in every package. It fails when a crate has no license file and its license has no standard text in `build/licenses`; add the text there. The Linux AppImage also carries the license files of the system libraries it bundles.

rawler, which reads raw camera files, is under the LGPL-2.1. Picasso is linked to it statically, so anyone who receives Picasso must be able to rebuild it with a changed rawler. Picasso's own source being public under the MIT license meets that. The license also asks that rawler's source be offered from the same place as the program, so the release job attaches the crate Picasso was built with, checked against `Cargo.lock`, as `rawler-<version>-source.tar.gz`. The notices say where to find both.
