# Contributing to niri-autostart

Thanks for contributing. This document is the source of truth for branches,
commits, pull requests, and releases.

History before `v0.1.12` used a different, ad-hoc workflow. Do not use it as a
style reference.

## Branch model

- `dev` is the integration branch. Feature and fix PRs always target `dev`.
- `main` is the release branch. It contains one squash commit per release and
  is updated only by release PRs from `dev`.
- Release tags use `vMAJOR.MINOR.PATCH` and point at the corresponding squash
  commit on `main`.

Never develop directly on `main`. Maintainers should not merge new work into
`dev` while a release PR is being merged and synchronized back.

## Contributor workflow

The examples use [`gh`](https://cli.github.com/), but the same branch and merge
rules apply when using another Git client.

1. Fork and clone the repository:

   ```sh
   gh repo fork partanskiy/niri-autostart --clone
   cd niri-autostart
   ```

   In this layout, `origin` is your fork and `upstream` is the canonical
   repository. If you already cloned upstream, `gh repo fork --remote` adds the
   fork without creating another clone.

2. Start from the latest `dev`:

   ```sh
   git fetch upstream dev
   git switch -c fix/short-description upstream/dev
   ```

3. Make focused commits, then run the local quality gate:

   ```sh
   cargo fmt --all -- --check
   cargo test --all-targets
   cargo clippy --all-targets -- -D warnings
   ```

4. Push the branch and open one PR into `dev`:

   ```sh
   git push -u origin fix/short-description
   gh pr create \
     --repo partanskiy/niri-autostart \
     --base dev \
     --title "fix: short description" \
     --body "Explain why the change is needed and what it does."
   ```

5. Push review fixes to the same branch. Avoid rewriting commits reviewers have
   already seen unless a maintainer asks for it.

## Commit messages

Development commits use
[Conventional Commits](https://www.conventionalcommits.org/). The common types
are:

| Type | Purpose |
| --- | --- |
| `feat:` | User-visible functionality |
| `fix:` | Bug fix |
| `docs:` | Documentation only |
| `test:` | Tests only |
| `refactor:` | Internal change without a behavior change |
| `perf:` | Performance improvement |
| `ci:` | CI or release automation |
| `chore:` | Tooling, dependency, or version maintenance |

Use an imperative, lowercase subject without a trailing period. Keep it around
72 characters or fewer. Add a body when the reason is not obvious from the
diff. Scopes such as `fix(ipc): ...` are welcome when useful.

Examples:

```text
fix: pin niri-ipc version
feat: add installation instructions
docs: document XDG runtime fallback
chore: bump to v0.3.2
```

## Pull request checklist

- [ ] The branch starts from current `dev`.
- [ ] The PR targets `dev`.
- [ ] Commits follow the message convention.
- [ ] User-visible behavior and paths are documented.
- [ ] Formatting, tests, and Clippy pass locally.

## Maintainer workflow

### Development PRs

Merge PRs into `dev` with **Rebase and merge**. This preserves their individual
commits and authors on the integration branch.

```sh
gh pr view <number>
gh pr checks <number>
gh pr merge <number> --rebase --delete-branch
```

Ask for noisy `wip` or review-fix commits to be cleaned up before merging.

### Releases

Releases move from `dev` to `main` through exactly one PR and use **Squash and
merge**. Do not merge other work into `dev` until step 5 is complete.

1. Choose the next version according to [SemVer](https://semver.org/). In a
   normal PR to `dev`, update `Cargo.toml` and `Cargo.lock` in a commit named:

   ```text
   chore: bump to vX.Y.Z
   ```

2. Confirm CI is green, then open the release PR:

   ```sh
   gh pr create \
     --base main \
     --head dev \
     --title "vX.Y.Z" \
     --body "Release vX.Y.Z."
   ```

3. Squash-merge it. The commit subject on `main` must be exactly the bare
   version:

   ```sh
   gh pr checks <number>
   gh pr merge <number> \
     --squash \
     --subject "vX.Y.Z" \
     --body ""
   ```

   Do not pass `--delete-branch`: `dev` is permanent.

4. Fetch the resulting release commit, verify its subject, and create an
   **annotated** tag:

   ```sh
   git fetch origin main
   git log -1 --format='%h %s' origin/main
   git tag -a vX.Y.Z origin/main -m "vX.Y.Z"
   git push origin vX.Y.Z
   ```

   Annotated tags are required because the AUR workflow resolves the peeled
   `refs/tags/v*.*.*^{}` ref. Never replace or force-push a published tag.

5. Immediately merge the release commit back into `dev`. The tree comparison
   must be empty because no new development was merged during the release:

   ```sh
   git switch dev
   git pull --ff-only origin dev
   git diff --exit-code HEAD origin/main
   git merge --no-ff origin/main -m "chore: sync main after vX.Y.Z release"
   git push origin dev
   ```

   Squash merge creates a new commit that exists only on `main`. This sync makes
   it an ancestor of `dev`, preventing old release changes from reappearing in
   the next release PR. It is release bookkeeping and is the only direct update
   allowed on `dev`.

6. Pushing the tag triggers `.github/workflows/release.yml`; a successful
   release then triggers `.github/workflows/aur.yml`. Verify both workflows,
   the release assets, and both AUR packages:

   ```sh
   gh run list --workflow release.yml --limit 1
   gh run list --workflow aur.yml --limit 1
   gh release view vX.Y.Z
   paru -Si niri-autostart niri-autostart-bin
   ```

The release is complete only after the GitHub release exists, both AUR entries
show the new version, and `main` is an ancestor of `dev`.
