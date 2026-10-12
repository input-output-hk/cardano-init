# Ecosystem: aikup, cardano-up, and friends

`cardano-init` creates a project once and then has no further role in it. Tools such as [`aikup`](https://github.com/aiken-lang/aikup) and [`cardano-up`](https://github.com/blinklabs-io/cardano-up) install and update the toolchains and services that the project uses. The two kinds of tool work together:

- `cardano-init` decides which tools the project uses and how they connect.
- `aikup`, `cardano-up`, and similar installers install and manage those tools.

They meet in the [`doctor`](commands.md#doctor) command. When a toolchain is missing, `doctor` tells you which installer to use, for example `aikup` for Aiken and `cardano-up` for the infrastructure role.

`cardano-init` does not:

- pin or upgrade tool versions;
- manage dependencies after the project is generated;
- migrate existing projects.

There is no `cardano-init update` command.
