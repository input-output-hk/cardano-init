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
 * Mesh's YaciProvider requires HTTP 202 for evaluation/submission, but Yaci Store 2.0.7
 * returns HTTP 200 to match Blockfrost. Keep both versions working until Mesh
 * accepts both statuses upstream. Also supplies the chain's cost models, which
 * Mesh's YaciProvider currently leaves unimplemented.
 */
export class YaciProvider extends MeshYaciProvider {
  private readonly apiUrl: string;

  constructor(url: string, adminUrl?: string) {
    super(url, adminUrl);
    this.apiUrl = url.replace(/\/+$/, "");
  }

  override async fetchCostModels(epoch?: number): Promise<number[][]> {
    const parameters = await this.get(`epochs/${epoch ?? "latest"}/parameters`);
    // Preserve ledger language order and each model's parameter order. Named
    // cost_models entries are not a substitute for the ordered raw arrays.
    return ["PlutusV1", "PlutusV2", "PlutusV3"].map((language) => {
      const model = parameters?.cost_models_raw?.[language];
      if (
        !Array.isArray(model) ||
        model.length === 0 ||
        !model.every((value: unknown) => Number.isSafeInteger(value))
      ) {
        throw new Error(`Yaci returned no valid cost model for ${language}`);
      }
      return model;
    });
  }

  private async postTx(
    endpoint: string,
    txHex: string,
    contentType: string,
  ): Promise<string> {
    const response = await fetch(`${this.apiUrl}/${endpoint}`, {
      method: "POST",
      headers: { "Content-Type": contentType },
      body: txHex,
      signal: AbortSignal.timeout(30_000),
    });
    const body = await response.text();
    if (response.status !== 200 && response.status !== 202) {
      throw new Error(`Yaci ${endpoint} failed (HTTP ${response.status}): ${body}`);
    }
    return body;
  }

  override async submitTx(txHex: string): Promise<string> {
    const body = await this.postTx("tx/submit", txHex, "text/plain");
    // Store returns a JSON string; older installations may return plain text.
    const hash: unknown = body.trim().startsWith('"') ? JSON.parse(body) : body.trim();
    if (typeof hash !== "string" || !/^[0-9a-fA-F]{64}$/.test(hash)) {
      throw new Error(`Yaci returned an invalid transaction hash: ${body}`);
    }
    return hash;
  }

  override async evaluateTx(txHex: string): Promise<Evaluation> {
    const body = await this.postTx("utils/txs/evaluate", txHex, "application/cbor");
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
