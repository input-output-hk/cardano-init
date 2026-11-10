# Releasing

Releases are built by [cargo-dist](https://opensource.axo.dev/cargo-dist/) (see
`.github/workflows/release.yml`) and triggered by pushing a version tag. The
release notes are sourced from `CHANGELOG.md`, which is generated from
[Conventional Commit](https://www.conventionalcommits.org/) history by
[git-cliff](https://git-cliff.org/) (see `cliff.toml`).

cargo-dist extracts the `CHANGELOG.md` section whose heading matches the tag and
places it at the **top** of the GitHub Release body, then appends its own
Install/Download tables below.

Each release builds 5 targets (x86_64/aarch64 Linux, x86_64/aarch64 macOS,
x86_64 Windows), attaches the archives, checksums, and the `shell`/`powershell`
installer scripts to the GitHub Release, and publishes the `cardano-init`
package to npm (needs the `NPM_TOKEN` repo secret).

## Cutting a release

`main` is protected by a ruleset that requires changes to land via pull
request, so the release prep goes through a PR and the tag is created on the
merged `main` commit — **not** pushed alongside a branch. The changelog must be
regenerated and committed **before** tagging, because cargo-dist reads the
committed file at tag-push time.

```bash
# 1. Branch off an up-to-date main.
git checkout main && git pull --ff-only
git checkout -b chore/release-vX.Y.Z

# 2. Bump the version in Cargo.toml (`version = "X.Y.Z"`) and sync the lockfile.
cargo update -p cardano-init

# 3. Turn the [Unreleased] section into [vX.Y.Z] and refresh links.
#    A GitHub token MUST be in the environment or every "by @handle" is dropped
#    (see the attribution note below). git-cliff via nix; drop the
#    `nix run nixpkgs#` prefix if it's installed.
GITHUB_TOKEN="$(gh auth token)" \
  nix run nixpkgs#git-cliff -- --config cliff.toml --tag vX.Y.Z --output CHANGELOG.md

# 4. Review CHANGELOG.md, then commit and open a PR.
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore(release): vX.Y.Z"
git push -u origin chore/release-vX.Y.Z
gh pr create --title "chore(release): vX.Y.Z" --body "Release prep for vX.Y.Z."

# 5. After the PR is merged, tag the merged commit on main and push the tag.
#    Pushing the tag is what triggers the release workflow.
git checkout main && git pull --ff-only
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin vX.Y.Z
```

> Do **not** `git push origin main vX.Y.Z` to release: the branch push is
> rejected by the ruleset while the tag slips through, triggering a release from
> a commit that isn't on `main`. Always merge the PR first, then tag `main`.

## Release candidates

Use a SemVer prerelease suffix: `vX.Y.Z-rc.N`, with the same suffix in
`Cargo.toml` (`version = "X.Y.Z-rc.N"`; cargo-dist requires the tag and the
crate version to match). Follow the steps above with that version everywhere.

- The GitHub Release is marked as a **prerelease** automatically.
- npm publishing is **skipped** for prereleases (`publish-prereleases` is not
  set in `dist-workspace.toml`), so `latest` on npm is never an RC.
- Don't tag the final version as an RC and flip it later: tag `vX.Y.Z` only
  for the final release.

`cliff.toml` sets `ignore_tags = "rc"`, so RC tags never get a changelog
section of their own once a later tag exists. Generating the RC itself works
as usual (`--tag vX.Y.Z-rc.N` names the unreleased section). When you
regenerate for the final `vX.Y.Z`, the RC section is replaced by a single
`vX.Y.Z` section covering every commit since the previous final release.

## Pipeline config (`dist-workspace.toml`)

`dist-workspace.toml` is the source of truth; `.github/workflows/release.yml` is
generated from it. **Never hand-edit `release.yml`**: CI runs
`dist generate --check` and fails if the two disagree.

- `targets`, `installers`, and `publish-jobs` define what gets built and where
  it is published.
- `cargo-dist-version` pins the `dist` version CI uses. Keep it equal to the
  `dist` you run locally (`cargo install cargo-dist --version <X> --locked`).
- Homebrew and crates.io publishing are off; the comment block in
  `dist-workspace.toml` lists the switches and credentials each one needs.

After changing the config (or bumping the pinned version), re-render the
workflow:

```bash
dist generate   # or `dist init` for an interactive config update
```

## Testing before you tag

Nothing here requires a push:

```bash
dist plan               # what artifacts would be produced (resolves all targets)
dist generate --check   # confirms release.yml matches dist-workspace.toml (the CI gate)
dist build --artifacts=local --target <your-host-triple>   # real compile + archive locally
```

To exercise the full build matrix in CI without publishing, set
`pr-run-mode = "upload"` in `dist-workspace.toml`, run `dist generate`, and open
a PR: CI builds all 5 targets and attaches them to the workflow run (no tag, no
Release). For a live end-to-end dry run, push a prerelease tag to a **fork** and
delete it afterward.

## Notes

- Only `feat`, `fix`, `perf`, `refactor`, `docs`, and `revert` commits appear in
  the changelog. `test`/`chore`/`ci`/`build`/`style` and merge commits are
  filtered out (see the `commit_parsers` in `cliff.toml`).
- Commit-message hygiene drives note quality. A doc-only change committed as
  `feat(docs): …` will be grouped under **Features**, not **Documentation** —
  the first matching parser (`^feat`) wins.
- Contributor `@handles` are resolved locally during generation, so regenerate
  the changelog locally (not in CI) to keep attribution.
- **Attribution needs a GitHub token.** The `by @handle` suffix comes from
  git-cliff's GitHub API integration, which is *silently skipped* when no token
  is present — you get a valid-looking changelog with every `by @…` missing, and
  no error. Always run generation with a token in the environment, e.g.
  `GITHUB_TOKEN="$(gh auth token)"` as shown in step 3. `cliff.toml` pins the
  repo (`[remote.github]`) so resolution no longer depends on the branch's
  upstream being set, but it still cannot invent the token. A handful of legacy
  commits authored with a non-GitHub email (e.g. `…@<host>.local`) have no handle
  to resolve and stay unattributed — that is expected, not a token problem.
  If you spot missing `@handles` in a fresh section, regenerate with the token
  before opening the release PR; if a release already published without them,
  fix `CHANGELOG.md` in a follow-up PR and `gh release edit <tag> --notes-file`
  to correct the published body.
