import { getBundledValidators, hasBundledBlueprint } from "./contract.js";
import {
  createGiftCardContractFromEnv,
  loadEnv,
  topupOnDevnet,
  walletAddress,
} from "./node.js";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** How long `redeem` waits for the gift-card UTxO to appear. */
const REDEEM_WAIT_MS = 180_000;

// Runnable entry point for `just dev` / `npm start` and the `create` / `redeem`
// commands. It submits real transactions when the blueprint is bundled and the
// required environment variables are present, and degrades to guidance
// otherwise. The blueprint is bundled from ../blueprint/plutus.json before this
// runs (the npm "pre" hooks call scripts/bundle-blueprint.mjs).
//
//   npm start                       # status: blueprint + env readiness
//   npx tsx src/cli.ts create <name> <lovelace>
//   npx tsx src/cli.ts redeem <redeemAddress>

function printStatus(): void {
  if (!hasBundledBlueprint()) {
    console.log("No blueprint bundled.");
    console.log(
      "Build the on-chain component first:  just -f ../on-chain/Justfile build",
    );
    return;
  }

  const { giftCard, redeem } = getBundledValidators();
  console.log("Bundled on-chain blueprint:");
  console.log(`  gift_card (mint):  ${giftCard.length} hex chars`);
  console.log(`  redeem    (spend): ${redeem.length} hex chars`);
  console.log("");

  const env = loadEnv();
  if (env.ok) {
    console.log("Environment ready.");
    console.log("Run a transaction:");
    console.log("  npx tsx src/cli.ts create <tokenName> <lovelace>");
    console.log("  npx tsx src/cli.ts redeem <redeemAddress>");
  } else {
    console.log("To submit transactions, set the following (see .env.example):");
    for (const key of env.missing) console.log(`  - ${key}`);
  }
}

function requireEnv() {
  if (!hasBundledBlueprint()) {
    throw new Error(
      "No blueprint bundled. Build on-chain first: " +
        "just -f ../on-chain/Justfile build",
    );
  }
  const env = loadEnv();
  if (!env.ok) {
    throw new Error(
      `Missing required configuration: ${env.missing.join(", ")}. ` +
        "See .env.example.",
    );
  }
  return env.env;
}

async function create(tokenName: string, lovelace: string): Promise<void> {
  const env = requireEnv();
  const { contract, blaze, provider } = await createGiftCardContractFromEnv({ env });

  // On a local devnet, top the wallet up from the faucet so a fresh wallet has
  // funds + a separate collateral UTxO. Amounts are in ADA. No-op on Blockfrost.
  const address = await walletAddress(blaze);
  const topped = await topupOnDevnet(provider, address, 10); // collateral
  if (topped) {
    await topupOnDevnet(provider, address, 10_000); // funds
    console.log(`Funded ${address.slice(0, 20)}… from the devnet faucet.`);
    // Wait until the faucet UTxOs are indexed before building.
    for (let i = 0; i < 60; i++) {
      const utxos = await blaze.wallet.getUnspentOutputs();
      if (utxos.length >= 2) break;
      await wait(1000);
    }
  }

  const { tx, redeemAddress } = await contract.createGiftCard(
    tokenName,
    BigInt(lovelace),
  );
  const txHash = await contract.signAndSubmit(tx);

  console.log(`Gift card "${tokenName}" created.`);
  console.log(`  tx:      ${txHash}`);
  console.log(`  address: ${redeemAddress}`);
  console.log("Once it is confirmed on chain, redeem it with:");
  console.log(`  npx tsx src/cli.ts redeem ${redeemAddress}`);
}

async function redeem(redeemAddress: string): Promise<void> {
  const env = requireEnv();
  const { contract } = await createGiftCardContractFromEnv({ env });

  // Wait for the create tx to be confirmed and indexed (a public testnet block
  // can take a minute or more), then locate the gift-card UTxO.
  const deadline = Date.now() + REDEEM_WAIT_MS;
  let giftCardUtxo = await contract.getGiftCardUtxoAt(redeemAddress);
  while (giftCardUtxo === undefined && Date.now() < deadline) {
    await wait(3000);
    giftCardUtxo = await contract.getGiftCardUtxoAt(redeemAddress);
  }
  if (giftCardUtxo === undefined) {
    throw new Error(
      `No gift-card UTxO found at ${redeemAddress} after ${REDEEM_WAIT_MS / 1000}s. ` +
        "Check that the create tx is confirmed, then retry.",
    );
  }

  const tx = await contract.redeemGiftCard(giftCardUtxo);
  const txHash = await contract.signAndSubmit(tx);

  console.log("Gift card redeemed; assets released to your wallet.");
  console.log(`  tx: ${txHash}`);
}

async function main(): Promise<void> {
  const [command, ...args] = process.argv.slice(2);

  switch (command) {
    case undefined:
    case "status":
      printStatus();
      break;
    case "create":
      if (args.length < 2) {
        throw new Error("usage: create <tokenName> <lovelace>");
      }
      await create(args[0]!, args[1]!);
      break;
    case "redeem":
      if (args.length < 1) {
        throw new Error("usage: redeem <redeemAddress>");
      }
      await redeem(args[0]!);
      break;
    default:
      throw new Error(`Unknown command "${command}". Use: status | create | redeem`);
  }
}

main()
  // The Kupmios provider holds an Ogmios WebSocket open; exit explicitly.
  .then(() => process.exit(0))
  .catch((err) => {
    console.error(err instanceof Error ? err.message : err);
    process.exit(1);
  });
