# README assets

## `demo.gif`

The animated demo shown at the top of the root `README.md` and the docs site.

It is recorded with [VHS](https://github.com/charmbracelet/vhs) from two tapes and joined with `ffmpeg`:

- [`demo.tape`](demo.tape): `cardano-init table`, then a one-shot scaffold (normal speed).
- [`demo-test.tape`](demo-test.tape): `just test` in the new project, played back at 4x, because the real run installs packages and boots a local devnet. The typed command says it is sped up.

```bash
# From the repository root, with cardano-init on PATH (e.g. target/debug)
assets/render-demo.sh

# On Nix, VHS and ffmpeg are one shell away
nix shell nixpkgs#vhs nixpkgs#ffmpeg -c assets/render-demo.sh
```

Requirements:

- the toolchains of the recorded stack (`aiken`, `node`, `just`, `yaci-devkit`, Docker), so `just test` passes. `cardano-init doctor` tells you what's missing;
- a [Nerd Font](https://www.nerdfonts.com) named `JetBrainsMono Nerd Font`, so the tool icons render. Without it installed system-wide, point `FONTCONFIG_FILE` at a config that includes the font's directory.

Keep the tapes as the source of truth (edit them, not the GIF) so the demo stays reproducible and reviewable in diffs.
