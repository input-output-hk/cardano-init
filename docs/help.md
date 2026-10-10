# Supporting tools and getting help

## Supporting tools

Beyond the toolchains themselves, each language has a layer of supporting tools: editor extensions (VS Code, Neovim, Zed, Emacs, JetBrains), language servers, and online playgrounds. You don't need to hunt for them: `cardano-init doctor` lists the ones that fit the stack it detects in your project, under *Supporting tools* (or as `deps[].support` with `--format json`), and `cardano-init list` shows them for every tool as a *Tooling* line (`tools[].support` in JSON). The links are kept as data in the `support` field of [`registry/deps.toml`](https://github.com/input-output-hk/cardano-init/blob/main/registry/deps.toml), sourced from each tool's own docs.

At the chain level, a few kinds of tool help with any stack: CBOR and transaction inspectors (to decode datums, redeemers, and raw transactions), block explorers for mainnet, preprod, and preview, and the testnet faucet for free test ada. The [developers.cardano.org tools directory](https://developers.cardano.org/tools/) is a good place to find them.

## Getting help

Every tool in the registry records its official community channels (usually a Discord server, sometimes GitHub Discussions or a forum) in the `community` field of `registry/tools/<tool>.toml`. `cardano-init doctor` shows them for the components it detects, under *Get help*, and `cardano-init list` shows them for every tool (as `tools[].community` under `--format json`). The channels are the ones each project links from its own website or repository.

For questions that aren't specific to one tool, the Cardano Foundation's developer channels (Discord, the Cardano Forum, and Cardano Stack Exchange) are linked from [developers.cardano.org](https://developers.cardano.org).
