# Releasing

Releases are built by [cargo-dist](https://opensource.axo.dev/cargo-dist/) (see
`.github/workflows/release.yml`) and triggered by pushing a version tag. The
release notes are sourced from `CHANGELOG.md`, which is generated from
[Conventional Commit](https://www.conventionalcommits.org/) history by
[git-cliff](https://git-cliff.org/) (see `cliff.toml`).

cargo-dist extracts the `CHANGELOG.md` section whose heading matches the tag and
places it at the **top** of the GitHub Release body, then appends its own
Install/Download tables below.

## Cutting a release

The changelog must be regenerated and committed **before** tagging, because
cargo-dist reads the committed file at tag-push time.

```bash
# 1. Pick the new version and bump Cargo.toml (`version = "X.Y.Z"`).
#    Keep Cargo.lock in sync:
cargo update -p cardano-init

# 2. Turn the [Unreleased] section into [X.Y.Z] and refresh links.
#    (git-cliff via nix; drop the `nix run nixpkgs#` prefix if it's installed.)
nix run nixpkgs#git-cliff -- --config cliff.toml --tag vX.Y.Z --output CHANGELOG.md

# 3. Review CHANGELOG.md, then commit.
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore(release): vX.Y.Z"

# 4. Tag and push. This is what triggers the release workflow.
git tag vX.Y.Z
git push origin main vX.Y.Z
```

## Notes

- Only `feat`, `fix`, `perf`, `refactor`, `docs`, and `revert` commits appear in
  the changelog. `test`/`chore`/`ci`/`build`/`style` and merge commits are
  filtered out (see the `commit_parsers` in `cliff.toml`).
- Commit-message hygiene drives note quality. A doc-only change committed as
  `feat(docs): …` will be grouped under **Features**, not **Documentation** —
  the first matching parser (`^feat`) wins.
- Contributor `@handles` are resolved locally during generation, so regenerate
  the changelog locally (not in CI) to keep attribution.
