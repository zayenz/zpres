# Releasing zpres

Update the version in `Cargo.toml` and `Cargo.lock`, update `CHANGELOG.md`, and
run the local checks in `AGENTS.md`. Commit and push the changes, then tag that
commit as `vMAJOR.MINOR.PATCH` and push the tag.

The `publish-crates.yml` workflow checks out the tag, checks that its version
matches Cargo, verifies the package, and publishes to crates.io. Only stable
version tags are accepted. It runs no browser checks.

## Authentication

The workflow uses [crates.io Trusted Publishing](https://crates.io/docs/trusted-publishing),
following zdev's release setup. In the
[zpres crate settings](https://crates.io/crates/zpres/settings), configure:

- Repository owner: `zayenz`
- Repository: `zpres`
- Workflow filename: `publish-crates.yml`
- Environment: leave empty

No permanent crates.io token or GitHub repository secret is needed.

## Retry a release

Run the workflow on `main`, selecting the existing release tag:

```sh
gh workflow run publish-crates.yml --ref main -f tag=v0.1.0
```

An already published version is skipped. A failed package check,
authentication, or publication fails the workflow.
