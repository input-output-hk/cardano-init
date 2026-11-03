import { once } from "node:events";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";

import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";

import { createProvider } from "./node.js";

const evaluation = {
  type: "jsonwsp/response",
  version: "1.0",
  servicename: "ogmios",
  methodname: "EvaluateTx",
  result: {
    EvaluationResult: {
      "mint:0": { memory: 34914, steps: 11964079 },
      "spend:1": { memory: 42000, steps: 13000000 },
    },
  },
};

describe("Yaci provider compatibility", () => {
  let status: number;
  let body: unknown;
  let baseUrl: string;
  let request: { method?: string; url?: string; type?: string; body: string };
  const server = createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(Buffer.from(chunk));
    request = {
      method: req.method,
      url: req.url,
      type: req.headers["content-type"],
      body: Buffer.concat(chunks).toString(),
    };
    res.writeHead(status, { "Content-Type": "application/json" });
    res.end(JSON.stringify(body));
  });

  beforeAll(async () => {
    await once(server.listen(0, "127.0.0.1"), "listening");
    baseUrl = `http://127.0.0.1:${(server.address() as AddressInfo).port}/api/v1`;
  });

  beforeEach(() => {
    status = 200;
    body = evaluation;
  });

  afterAll(async () => {
    await new Promise<void>((resolve, reject) => {
      server.close((error) => (error ? reject(error) : resolve()));
      server.closeAllConnections();
    });
  });

  it.each([undefined, 0, 42])("fetches ordered cost models for epoch %s", async (epoch) => {
    body = {
      cost_models_raw: {
        PlutusV3: [30, -31, 32],
        PlutusV1: [10, 11],
        PlutusV2: [20, 21],
      },
    };
    const provider = createProvider({ kind: "yaci", url: `${baseUrl}/` });
    await expect(provider.fetchCostModels(epoch)).resolves.toEqual([
      [10, 11], [20, 21], [30, -31, 32],
    ]);
    expect(request.method).toBe("GET");
    expect(request.url).toBe(`/api/v1/epochs/${epoch ?? "latest"}/parameters`);
  });

  it.each([undefined, [], [1, "2"]])("rejects missing or malformed cost models: %j", async (model) => {
    body = { cost_models_raw: { PlutusV1: [1], PlutusV2: [2], PlutusV3: model } };
    const provider = createProvider({ kind: "yaci", url: baseUrl });
    await expect(provider.fetchCostModels()).rejects.toThrow(
      "Yaci returned no valid cost model for PlutusV3",
    );
  });

  it.each([200, 202])("accepts evaluation budgets with HTTP %i", async (code) => {
    status = code;
    const provider = createProvider({ kind: "yaci", url: `${baseUrl}/` });
    await expect(provider.evaluateTx("84a700")).resolves.toEqual([
      { tag: "MINT", index: 0, budget: { mem: 34914, steps: 11964079 } },
      { tag: "SPEND", index: 1, budget: { mem: 42000, steps: 13000000 } },
    ]);
    expect(request).toEqual({
      method: "POST",
      url: "/api/v1/utils/txs/evaluate",
      type: "application/cbor",
      body: "84a700",
    });
  });

  it.each([200, 202])("accepts a submitted transaction hash with HTTP %i", async (code) => {
    status = code;
    body = "65d9e40ff443d23d6d5d23c49ad8a041a27f6cc3c74bfbc5cb7f79923942d694";
    const provider = createProvider({ kind: "yaci", url: baseUrl });
    await expect(provider.submitTx("84a700")).resolves.toBe(body);
    expect(request).toEqual({
      method: "POST",
      url: "/api/v1/tx/submit",
      type: "text/plain",
      body: "84a700",
    });
  });

  it("rejects a successful response without a valid transaction hash", async () => {
    body = { error: "submission failed" };
    const provider = createProvider({ kind: "yaci", url: baseUrl });
    await expect(provider.submitTx("84a700")).rejects.toThrow(
      "Yaci returned an invalid transaction hash",
    );
  });

  it("preserves submission failures", async () => {
    status = 400;
    body = { error: "transaction rejected" };
    const provider = createProvider({ kind: "yaci", url: baseUrl });
    await expect(provider.submitTx("84a700")).rejects.toThrow(
      'Yaci tx/submit failed (HTTP 400): {"error":"transaction rejected"}',
    );
  });

  it.each([400, 500])("preserves evaluation errors with HTTP %i", async (code) => {
    status = code;
    body = { error: "script validation failed" };
    const provider = createProvider({ kind: "yaci", url: baseUrl });
    await expect(provider.evaluateTx("84a700")).rejects.toThrow(
      `Yaci utils/txs/evaluate failed (HTTP ${code}): {"error":"script validation failed"}`,
    );
  });

  it.each([200, 202])("rejects missing budgets even with HTTP %i", async (code) => {
    status = code;
    body = { result: { EvaluationFailure: { "mint:0": "validator failed" } } };
    const provider = createProvider({ kind: "yaci", url: baseUrl });
    await expect(provider.evaluateTx("84a700")).rejects.toThrow(
      "Yaci returned no evaluation budgets",
    );
  });

  it.each([
    { "unknown:0": { memory: 1, steps: 2 } },
    { "mint:invalid": { memory: 1, steps: 2 } },
    { "mint:0": { memory: -1, steps: 2 } },
    { "mint:0": { memory: "34914", steps: 2 } },
    { "mint:0": null },
  ])("rejects malformed budgets: %j", async (result) => {
    body = { result: { EvaluationResult: result } };
    const provider = createProvider({ kind: "yaci", url: baseUrl });
    await expect(provider.evaluateTx("84a700")).rejects.toThrow(
      "Yaci returned an invalid evaluation budget",
    );
  });
});
