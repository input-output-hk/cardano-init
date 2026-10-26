import { YaciProvider as MeshYaciProvider } from "@meshsdk/core";

type Evaluation = Awaited<ReturnType<MeshYaciProvider["evaluateTx"]>>;

const tags = {
  spend: "SPEND",
  mint: "MINT",
  certificate: "CERT",
  publish: "CERT",
  withdrawal: "REWARD",
  reward: "REWARD",
  vote: "VOTE",
  propose: "PROPOSE",
} as const;

/**
 * Mesh's YaciProvider requires HTTP 202 for evaluation, but Yaci Store 2.0.7
 * returns HTTP 200 to match Blockfrost. Keep both versions working until Mesh
 * accepts both statuses upstream. All other provider operations are inherited.
 */
export class YaciProvider extends MeshYaciProvider {
  private readonly evaluationUrl: string;

  constructor(url: string, adminUrl?: string) {
    super(url, adminUrl);
    this.evaluationUrl = `${url.replace(/\/+$/, "")}/utils/txs/evaluate`;
  }

  override async evaluateTx(txHex: string): Promise<Evaluation> {
    const response = await fetch(this.evaluationUrl, {
      method: "POST",
      headers: { "Content-Type": "application/cbor" },
      body: txHex,
      signal: AbortSignal.timeout(30_000),
    });
    const body = await response.text();
    if (response.status !== 200 && response.status !== 202) {
      throw new Error(`Yaci evaluation failed (HTTP ${response.status}): ${body}`);
    }

    const data = JSON.parse(body);
    const result = data?.result?.EvaluationResult;
    if (!result || typeof result !== "object" || Array.isArray(result)) {
      throw new Error(`Yaci returned no evaluation budgets: ${body}`);
    }

    return Object.entries(result).map(([key, value]) => {
      const [purpose, index, extra] = key.split(":");
      const tag = tags[purpose as keyof typeof tags];
      const budget = value as { memory?: unknown; steps?: unknown } | null;
      if (
        !Object.hasOwn(tags, purpose) ||
        !/^\d+$/.test(index ?? "") ||
        !Number.isSafeInteger(Number(index)) ||
        extra !== undefined ||
        !budget ||
        typeof budget.memory !== "number" ||
        !Number.isSafeInteger(budget.memory) ||
        budget.memory < 0 ||
        typeof budget.steps !== "number" ||
        !Number.isSafeInteger(budget.steps) ||
        budget.steps < 0
      ) {
        throw new Error(`Yaci returned an invalid evaluation budget: ${key}`);
      }
      return {
        tag,
        index: Number(index),
        budget: { mem: budget.memory, steps: budget.steps },
      };
    });
  }
}
