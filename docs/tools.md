# Tools catalog

Tools currently in the registry (✅ available · 🧪 experimental · ⬜ planned):

| On-chain | Off-chain | Devnet | Infrastructure | Formal methods |
|----------|-----------|--------|----------------|----------------|
| ✅ Aiken | ✅ MeshJS | ✅ Yaci DevKit | ✅ Kupo | 🧪 Blaster |
| ✅ Scalus | ✅ Scalus | | ✅ Ogmios | |
| ✅ Plinth | ✅ Evolution SDK | | ✅ Dolos | |
| ⬜ Pebble | 🧪 Tx3 | | ✅ Tx Submit API | |
| ⬜ Plutarch | ⬜ Lucid Evolution | | ✅ Cardano Node | |
| ⬜ Opshin | ⬜ Blaze | | ✅ Cardano Node API | |
| | ⬜ Elm Cardano | | ✅ Dingo | |
| | ⬜ PyCardano | | | |

This table can be out of date. `list` shows the tools in the version you have installed:

```bash
cardano-init list           # every tool with its id, language, status, and links
cardano-init list --table   # the same matrix as above
```

Use the **tool id** shown by `list` (for example `aiken`, `meshjs`, `yaci`, `tx-submit-api`) as the value for role flags.

## Notes

- **Scalus** fills both on-chain and off-chain and supports `--fullstack`, which generates a single `protocol/` component. See [Fullstack tools](how-it-works.md#fullstack-tools).
- **Experimental** tools (🧪) require `--allow-experimental` in one-shot mode. See [Experimental tools](how-it-works.md#experimental-tools).
- **Infrastructure** is the only role that accepts several tools. See [Infrastructure providers](infrastructure.md).
- To add a tool to the registry, see [Adding a tool](ADDING_A_TOOL.md).
