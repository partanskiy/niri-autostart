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

The flow below uses [`gh`](https://cli.github.com/) end-to-end. If you do not
have `gh` installed, the equivalent web-UI steps work too — the rules
(branch off `dev`, target `dev`, follow the commit style) are what matter.

1. Fork the repository and clone your fork. `gh repo fork --clone` does both
   in one step and sets `upstream` to point at the canonical repository:

   ```sh
   gh repo fork partanskiy/niri-autostart --clone
   cd niri-autostart
   ```

   If you already have a clone of the upstream, run `gh repo fork --remote`
   from inside it instead — it adds your fork as a remote without recloning.

2. Branch off `dev` (sync from upstream first):

   ```sh
   git switch dev
   git pull upstream dev
   git switch -c feat/short-description
   ```

3. Make your changes. Keep commits small and well-named (see
   [Commit messages](#commit-messages)). Multiple commits per PR are fine
   and encouraged — they are preserved on `dev`.

4. Push the branch to your fork and open a pull request against
   `partanskiy/niri-autostart:dev`:

   ```sh
   git push -u origin feat/short-description
   gh pr create \
     --repo partanskiy/niri-autostart \
     --base dev \
     --title "feat: short description" \
     --body  "Why this change is needed and what it does."
   ```

   `gh pr create --web` opens the prefilled PR form in the browser if you
   prefer to write the description there.

5. While review is in progress:

   ```sh
   gh pr status                       # see your PR's review/CI state
   gh pr checks                       # tail CI results
   gh pr view --web                   # open the PR in the browser
   ```

   Push follow-up commits to the same branch — the PR updates automatically.
   Do not force-push to rewrite history that reviewers have already seen
   unless asked; a maintainer will tidy commits at merge time if needed.

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

Full `gh` flow:

```sh
gh pr view <num>                        # sanity-check title, base, commits
gh pr checks <num>                      # confirm CI is green
gh pr review <num> --approve            # optional: leave an approval
gh pr merge  <num> --rebase --delete-branch
```

`--rebase` is the squash/merge/rebase selector — it must be `--rebase` for
PRs into `dev`. `--delete-branch` removes the contributor's topic branch on
the fork side after the merge.

### Release process

Releases are always cut from `dev` into `main` and are **always** merged
with **Squash and merge** — no exceptions.

1. On `dev` (via a normal PR), bump the version in `Cargo.toml` and refresh
   `Cargo.lock` with a commit named `chore: bump to vX.Y.Z`.

2. Open a release PR `dev` → `main`:

   ```sh
   gh pr create \
     --base main \
     --head dev \
     --title "vX.Y.Z" \
     --body  "Release vX.Y.Z."
   ```

   The PR title is set to the bare version on purpose — see step 3.

3. Merge the release PR with **Squash and merge**. The squash commit title
   must be exactly the version, e.g. `v0.1.12` — no prefix, no extra words.
   This is the only commit style on `main`. Use `gh` to enforce both the
   strategy and the subject:

   ```sh
   gh pr merge <num> \
     --squash \
     --subject "vX.Y.Z" \
     --body    ""
   ```

   Do **not** pass `--delete-branch` — `dev` must survive the merge.

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
   `.github/workflows/aur.yml` updates the AUR packages. Watch the run with:

   ```sh
   gh run watch
   gh release view vX.Y.Z
   ```

Notes:

- Versioning follows [SemVer](https://semver.org/): bump MAJOR for breaking
  changes, MINOR for new features, PATCH for bug fixes.
- The tag name and the squash commit title must match exactly.
