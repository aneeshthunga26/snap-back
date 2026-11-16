# CI and releases

GitHub Actions runs CI on hosted Ubuntu 24.04 runners, using an Arch Linux container for builds. This matches Snap Back's CachyOS/Arch dependencies while keeping CI independent of the development VM. Local development and release-packaging checks continue to run inside `cachy-vm`.

## What runs

Every push to `main` and every pull request checks formatting, runs the Rust unit tests, and runs Clippy with warnings treated as errors. A push to `main`, or a manual CI run, additionally builds release binaries, packages the binary and source tarballs, verifies their checksums, and checks an installation staged under a temporary directory.

The tarballs and `SHA256SUMS` are available from the completed CI run's **Artifacts** section for 14 days. GitHub requires sign-in for workflow artifact downloads. Versioned [Releases](https://github.com/aneeshthunga26/snap-back/releases) provide the public downloads.

Pull requests, including forks, run on GitHub's isolated hosted runners with read-only workflow permissions. They do not publish releases or receive write credentials. There is no `pull_request_target` workflow, and the automatic checks never run the destructive Btrfs or offline-recovery test scripts.

## Publish a version

Update the workspace version and `Cargo.lock`, commit the change, then push a matching tag:

```sh
git tag v0.1.1
git push origin v0.1.1
```

The release workflow repeats formatting, tests, and Clippy, then calls `packaging/release.sh` with the tag. A tag whose version does not match the Cargo workspace fails before publication. The workflow verifies the archives before handing them to a separate publishing job.

The publishing job creates a draft release, uploads the Linux x86_64 tarball, corresponding source tarball, and checksums, then publishes it. A failed upload leaves a draft that can be retried. An already published version is never overwritten; fixes need a new version and tag. Tags containing a prerelease suffix, such as `v0.2.0-rc.1`, produce prereleases.

## Build environment

Build jobs use the official `archlinux:base` container. The bootstrap step updates its packages and installs Git, `base-devel`, Rust, pkg-config, Wayland/XKB libraries, font libraries, Python, tar, and gzip before checkout. Arch's `rust` package includes Cargo, rustfmt, and Clippy. Build steps explicitly trust their checked-out workspace so Git can read source metadata across the container's ownership boundary. Official JavaScript actions run through the hosted runner's bundled Node runtime.

Cargo output stays in the checkout's ignored `target/` directory; Cargo's dependency cache stays outside the checkout in the container's home directory. Workflows limit Cargo to two build jobs to keep memory use bounded. No backup drives, backup configuration, desktop session, or system-recovery privileges are needed for CI.

The publishing job runs directly on a hosted Ubuntu runner. It downloads and verifies the already built archives, then uploads them to GitHub Releases; it does not compile the project. No self-hosted runner or running development VM is required.

Actions are pinned to reviewed commit hashes. Checkout does not retain its GitHub credentials. Default workflow permissions are read-only; only the release publishing job receives `contents: write`. No personal access token is stored in the repository: publication uses the workflow's short-lived `GITHUB_TOKEN`.
