import { createServer } from "node:http";

// A tiny HTTP server that records every request it receives. Used to assert
// that pingward actually delivered a webhook (test-send and down/up delivery).
// It always answers 200 so the notifier records a successful send.
export async function startMockServer() {
  const requests = [];
  const server = createServer((req, res) => {
    let body = "";
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => {
      requests.push({
        method: req.method,
        path: req.url,
        headers: req.headers,
        body,
      });
      res.statusCode = 200;
      res.end("ok");
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  const url = `http://127.0.0.1:${port}`;

  // Poll the recorded requests until one satisfies `predicate` (or time out).
  // Delivery is fire-and-forget in pingward, so the POST arrives shortly after
  // the ping response, not synchronously.
  async function waitForRequest(predicate, timeoutMs = 5000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const match = requests.find(predicate);
      if (match) return match;
      await new Promise((r) => setTimeout(r, 50));
    }
    throw new Error("mock server did not receive a matching request in time");
  }

  // server.close() only stops accepting new connections and waits for existing
  // (idle keep-alive) ones to end, which can stall teardown until Node's ~5s
  // keepAliveTimeout. pingward's reqwest client pools keep-alive connections, so
  // force-close them to make cleanup prompt. closeAllConnections is Node >=18.2.
  const cleanup = () =>
    new Promise((resolve) => {
      server.closeAllConnections?.();
      server.close(() => resolve());
    });

  return { url, requests, waitForRequest, cleanup };
}
