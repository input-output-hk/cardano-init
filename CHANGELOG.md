# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [v0.4.1] - 2026-11-12

### Documentation

- Clarify tool status and list tested combinations by @rober-m
- List which platform tests each doctor install command by @rober-m
- Add a manual release-testing checklist by @rober-m

### Fixes

- Emit CIP-57-conformant blueprint schemas by @rober-m
- Target protocol 10 builtins so MeshJS can apply parameters by @rober-m
- Use nixpkgs' ghc96 for the ghc nix recipe by @rober-m
- Drop the nix recipe for rust by @rober-m
- Stop interactive prompts erasing lines and show the cursor by @rober-m
- Restore the cursor when Ctrl+C interrupts a prompt by @rober-m

## [v0.4.0] - 2026-11-10

### Documentation

- Link the upstream wrapper issue from the devnet teardown by @rober-m
- Refresh README, user guide, and demo GIF by @rober-m
- Re-record the demo gif at the larger zoom by @rober-m

### Features

- Replace --format <human|json> with --json by @paulobressan
- Lucid evolution (#109) by @paulobressan
- Add Blaze off-chain template (#114) by @paulobressan
- Add Plutarch on-chain template (#113) by @rober-m
- Check minimum dependency versions by @rober-m
- Tool icons and a role-grouped `list` by @rober-m
- Show the tool icon in place of the language in the summary by @rober-m
- Add PyCardano off-chain template by @paulobressan
- Add `table` command and shorten Lucid Evolution id to `lucid` by @rober-m
- Add Apollo off-chain template by @rober-m
- Add Pebble on-chain template (experimental) by @rober-m

### Fixes

- Restore Yaci devnet smoke compatibility (#110) by @rober-m
- Require the yaci-devkit CLI for Yaci projects (#117) by @rober-m
- Refresh Yaci DevKit components after an upgrade by @rober-m
- Report corrupt blueprints and low funds clearly in PyCardano template by @paulobressan
- Validate gift-card UTxOs and require Yaci for the e2e test by @rober-m
- Stop the whole devnet on teardown by @rober-m
- Start the devnet with setsid so teardown works without a TTY by @rober-m
- Wait for the faucet before running devnet tests (#128) by @rober-m
- Zoom in on the demo gif by @rober-m

### Refactor

- Clean up structure, duplication and comments across modules (#107) by @paulobressan
- Use PyCardano helpers and validate gift card UTxOs by @paulobressan
- Address PyCardano template review feedback by @paulobressan
- Make the Go template idiomatic by @rober-m

## [v0.3.0] - 2026-10-12

### Documentation

- Explain supporting tools and help channels instead of listing links by @paulobressan
- Tidy DepRecipe struct comment by @paulobressan
- Publish usage docs site with mdBook on GitHub Pages by @paulobressan
- Align usage docs with developer portal writing rules by @paulobressan
- Organize docs into user-guide, contributing, and design sections by @paulobressan
- Hide page headings in the sidebar by @paulobressan

### Features

- Return unsigned Evolution gift-card transactions by @0xBora
- Add flag aliases and short flags by @paulobressan
- List supporting tools and community channels by @paulobressan
- Show community and supporting tool links by @paulobressan
- Add pre-generation version-update notice by @paulobressan
- Cache the version-update check once a day by @paulobressan
- Generate a spec/ protocol specification folder (--no-spec to skip) by @paulobressan

### Fixes

- Avoid logging environment values (#81) by @wolf31o2
- Restore contributor attribution in release notes by @rober-m
- Report missing Cardano project and list all installed tooling by @paulobressan
- Show only the project's required tooling inside a project by @paulobressan
- Link spec/ instead of a possibly missing giftcard.md in AGENTS.md by @paulobressan

### Refactor

- Resolve the update-check cache dir with dirs by @paulobressan
- Simplify install-method detection by @paulobressan

## [v0.2.1] - 2026-08-20

### Documentation

- Keep v-prefix in changelog headings and document PR-based release flow (#73) by @rober-m
- Refresh README with a stronger hook and demo GIF (#76) by @rober-m

### Features

- Add --table for a compact tools-by-role matrix (#77) by @rober-m

### Fixes

- Detect tx3 despite @meshsdk dep, and silence git note (#74) by @rober-m

## [v0.2.0] - 2026-08-18

### Documentation

- Add installer-recipes CI badge to README by @rober-m
- Document the Gift Card reference example by @rober-m
- Correct inaccuracies vs implementation by @rober-m

### Features

- Add `fullstack` option that combines `on-chain` and `off-chain` roles in a single folder by @rober-m
- Implement Scalus on-chain, off-chain, and fullstack templates by @rober-m
- Add scalus to smoke test CI by @rober-m
- Add CI to verify dependency installers by @rober-m
- Clean pychache and allow running CI on selected PRs by @rober-m
- Add Plinth + restructure --nix generation to have composable flakes by @rober-m
- Make direnv allow non-interactive with a warning for non-trusted substituters by @rober-m
- Change validator to match default "Gift Card" example by @rober-m
- Gate experimental tools behind explicit opt-in (#11) by @rober-m
- Scalus templates use the Gift Card example, blueprint-driven off-chain by @rober-m
- Add Evolution SDK by @rober-m
- Data-driven off-chain ↔ provider compatibility by @rober-m
- Add Tx3 off-chain template (Gift Card) by @rober-m
- Remove unnecessary comments by @rober-m
- Generate AGENTS.md tailored to the selected stack by @rober-m
- Redesign CLI output and errors by @rober-m
- Add devnet smoke badge to README by @rober-m
- First draft on update/remove tooling by @rober-m
- Add git initialization, remove `cardano-init edit`, and improve help message by @rober-m
- `cardano-init add` replaces without prompt if git tree is clean by @rober-m
- Fold updating-project-ooling to main docs by @rober-m
- Fold updating-project-ooling to main docs by @rober-m
- Add more tests about updating tools functionality by @rober-m
- Fix network to preview, switch via CARDANO_NETWORK in .env by @rober-m
- Drop the web builder surface by @rober-m

### Fixes

- Don't crash installer verify on unstattable bin dir by @rober-m
- Correct broken doctor recipes and harden installer-recipes gate by @rober-m
- Detect tx3 off-chain (#58) by @rober-m
- Add FromStr impl for Network to fix clippy build by @rober-m
- Canonicalize nix_packages order (#65) by @rober-m

### Refactor

- Void datum + off-chain seed, dropping the redundant params by @rober-m

## [v0.1.0] - 2026-07-10

### Features

- Add `cardano-init doctor` by @rober-m
- Add flake.nix by @rober-m
- Add `cardano-init list` command by @rober-m
- Update README by @rober-m
- Add nix installer (closes #2) by @rober-m
- Improve install/usage instructions by @rober-m
- Update tests and docs by @rober-m
- Add cargo dist to hanble the releases
- Ignore .claude by @rober-m
- Added npm to publish platforms by @rober-m

### Fixes

- Remove redundant borrow in println (clippy 1.97) by @paulobressan

[v0.4.1]: https://github.com/input-output-hk/cardano-init/compare/v0.4.0..v0.4.1
[v0.4.0]: https://github.com/input-output-hk/cardano-init/compare/v0.3.0..v0.4.0
[v0.3.0]: https://github.com/input-output-hk/cardano-init/compare/v0.2.1..v0.3.0
[v0.2.1]: https://github.com/input-output-hk/cardano-init/compare/v0.2.0..v0.2.1
[v0.2.0]: https://github.com/input-output-hk/cardano-init/compare/v0.1.0..v0.2.0
[v0.1.0]: https://github.com/input-output-hk/cardano-init/releases/tag/v0.1.0

