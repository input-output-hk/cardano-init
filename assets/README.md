# README assets

## `demo.gif`

The animated demo shown at the top of the root `README.md`.

It is generated from [`demo.tape`](demo.tape) with [VHS](https://github.com/charmbracelet/vhs):

```bash
# Install VHS (see the repo for platform packages)
go install github.com/charmbracelet/vhs@latest   # or: brew install vhs / nix run nixpkgs#vhs

# Regenerate assets/demo.gif from the tape
vhs assets/demo.tape
```

Run it in an environment where the demo's toolchains are installed (`aiken`,
`node`, `just`, and Yaci DevKit), so the recorded `just test` actually passes.
`cardano-init doctor` will tell you what's missing.

Keep the tape as the source of truth — edit `demo.tape`, not the GIF — so the
demo stays reproducible and reviewable in diffs.
