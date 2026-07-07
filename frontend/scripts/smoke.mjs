import { spawn } from "node:child_process";
import { request } from "node:http";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const PORT = 3001;
const BASE_URL = `http://localhost:${PORT}`;
const TIMEOUT_MS = 60000;
const POLL_INTERVAL_MS = 1000;

function waitForServer(url, timeoutMs) {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    function poll() {
      if (Date.now() - start > timeoutMs) {
        reject(new Error(`Server did not respond within ${timeoutMs}ms`));
        return;
      }
      const req = request(url, { method: "HEAD" }, (res) => {
        resolve(res.statusCode);
      });
      req.on("error", () => {
        setTimeout(poll, POLL_INTERVAL_MS);
      });
      req.end();
    }
    poll();
  });
}

function fetchUrl(url) {
  return new Promise((resolve, reject) => {
    request(url, { method: "GET" }, (res) => {
      let data = "";
      res.on("data", (chunk) => (data += chunk));
      res.on("end", () =>
        resolve({ statusCode: res.statusCode, body: data })
      );
    }).on("error", reject).end();
  });
}

async function main() {
  const frontendDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const server = spawn("npm", ["run", "start"], {
    cwd: frontendDir,
    stdio: ["ignore", "pipe", "pipe"],
    shell: true,
  });

  let serverOutput = "";
  server.stdout.on("data", (chunk) => (serverOutput += chunk.toString()));
  server.stderr.on("data", (chunk) => (serverOutput += chunk.toString()));

  let exitCode = 1;
  try {
    const statusCode = await waitForServer(BASE_URL, TIMEOUT_MS);
    if (statusCode !== 200) {
      console.error(`Server returned status ${statusCode}, expected 200`);
      process.exit(1);
    }

    const { statusCode: getStatus, body } = await fetchUrl(BASE_URL);
    if (getStatus !== 200) {
      console.error(`GET / returned status ${getStatus}, expected 200`);
      process.exit(1);
    }

    if (!body.includes("<!DOCTYPE html>") && !body.includes("<html")) {
      console.error("Response does not contain HTML");
      process.exit(1);
    }

    console.log("Smoke test passed: server is running and serving HTML");
    exitCode = 0;
  } catch (err) {
    console.error("Smoke test failed:", err.message);
    console.error("Server output:", serverOutput);
  } finally {
    server.kill("SIGTERM");
    setTimeout(() => {
      server.kill("SIGKILL");
    }, 2000);
  }

  process.exit(exitCode);
}

main();
