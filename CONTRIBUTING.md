# Contributing to niri-autostart

Thanks for taking the time to contribute. This document describes the
branching model, commit style, and release flow used in this repository.

> History before `v0.1.12` used a different, ad-hoc flow (direct commits to
> `main`, inline version bumps). Everything from `v0.1.12` onward follows the
> rules below — please ignore the older history as a style reference.

## Branches

- **`main`** — release branch. Contains only released versions; one commit
  per tagged release (`v*.*.*`). Contributors do not interact with `main` —
  releases are cut by maintainers (see [Releases](#release-process-maintainers)).
- **`dev`** — integration branch. All real development happens here. All
  contributor PRs target `dev`.

## Contributor workflow

1. Branch off `dev`:

   ```sh
   git switch dev
   git pull
   git switch -c feat/short-description
   ```

2. Make your changes. Keep commits small and well-named (see
   [Commit messages](#commit-messages)). Multiple commits per PR are fine
   and encouraged — they are preserved on `dev`.

3. Open a pull request against `dev`.

## Commit messages

Your commits are preserved verbatim on `dev` (maintainers merge with rebase,
not squash), so author and message stay yours. Use
[Conventional Commits](https://www.conventionalcommits.org/):

| Prefix      | Use for                                                    |
| ----------- | ---------------------------------------------------------- |
| `feat:`     | user-visible new functionality                             |
| `fix:`      | bug fix                                                    |
| `chore:`    | tooling, deps, version bumps, no behavior change           |
| `refactor:` | internal restructuring, no behavior change                 |
| `docs:`     | documentation only                                         |
| `ci:`       | GitHub Actions / pipelines                                 |
| `test:`     | tests only                                                 |
| `perf:`     | performance change with no functional difference           |

Rules:

- Subject in imperative mood, lowercase after the prefix, no trailing
  period.
- Keep the subject under ~72 characters.
- If a scope helps, use `feat(ipc): ...` style.
- Put the "why" in the body when it is not obvious from the diff.

Examples (real commits from the repo):

```
fix: Pin niri-ipc version
fix: Update for compatibility with Niri 26.04
feat: Add installation section to README
chore: bump to v0.1.12
```

## Local development

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

Please make sure `cargo fmt` and `cargo clippy` are clean before opening a
PR.

## Pull request checklist

- [ ] Branched off latest `dev`.
- [ ] Commits follow the [commit message](#commit-messages) style.
- [ ] `cargo fmt`, `cargo clippy`, `cargo test` pass locally.

---

## Maintainers

This section is for repository maintainers. Contributors do not need to
follow it.

### Merging into `dev`

Merge contributor PRs into `dev` with **Rebase and merge**. Rebase keeps the
original commits and their authorship, which is why we ask contributors to
write Conventional Commit messages — they end up on `dev` unchanged.

If a PR has noisy commits ("wip", "fix typo", "address review") ask the
author to clean them up (or do it yourself) before rebasing.

### Release process

Releases are always cut from `dev` into `main` and are **always** merged
with **Squash and merge** — no exceptions.

1. On `dev` (via a normal PR), bump the version in `Cargo.toml` and refresh
   `Cargo.lock`:

   ```
   chore: bump to vX.Y.Z
   ```

2. Open a PR `dev` → `main`.

3. Merge it with **Squash and merge**. The squash commit title must be
   exactly the version, e.g. `v0.1.12` — no prefix, no extra words. This is
   the only commit style on `main`.

4. Tag the resulting squash commit on `main` with the same name and push
   the tag:

   ```sh
   git switch main
   git pull
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

5. Pushing the tag triggers `.github/workflows/release.yml`, which builds
   the binaries and creates the GitHub release. On success,
   `.github/workflows/aur.yml` updates the AUR packages.

Notes:

- Versioning follows [SemVer](https://semver.org/): bump MAJOR for breaking
  changes, MINOR for new features, PATCH for bug fixes.
- The tag name and the squash commit title must match exactly.
