# Ecosystem: aikup, cardano-up, and friends


`cardano-init` is a **project scaffolder**, not a version manager or an environment manager. It runs once, generates a wired-together monorepo, and steps out. That makes it complementary to (not a replacement for) the per-tool installers in the ecosystem.

These sit at different layers: `cardano-init` decides *what tools your project uses and how they compose*, while `aikup` / `cardano-up` install and manage *the toolchains and infrastructure those tools need*. The two meet at the dependency [`doctor`](commands.md#doctor): when toolchains are missing, `cardano-init` advises the right installer (`aikup` for Aiken, `cardano-up` for the infrastructure role) rather than reinventing them.

By design, `cardano-init` is **not** a package or version manager: it does not pin or upgrade tool versions, manage dependencies after generation, or migrate existing projects. There is no `cardano-init update`.

