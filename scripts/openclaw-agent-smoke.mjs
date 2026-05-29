#!/usr/bin/env node

import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { resolve } from "node:path";

const DEFAULT_FIRST_MESSAGE = "Reply exactly: WECODE_GATEWAY_DIRECT_OK";
const DEFAULT_SECOND_MESSAGE = "Reply exactly: WECODE_GATEWAY_RESUME_OK";
const DEFAULT_TIMEOUT_MS = 180_000;

function parseArgs(argv) {
  const options = {
    configPath:
      process.env.WECODE_OPENCLAW_CONFIG_PATH ||
      process.env.OPENCLAW_CONFIG_PATH ||
      "~/.wecode/openclaw-state/openclaw.json",
    firstMessage: DEFAULT_FIRST_MESSAGE,
    secondMessage: DEFAULT_SECOND_MESSAGE,
    sessionKey: `agent:main:wecode-smoke-${Date.now()}`,
    timeoutMs: DEFAULT_TIMEOUT_MS,
  };

  for (let idx = 0; idx < argv.length; idx += 1) {
    const arg = argv[idx];
    switch (arg) {
      case "--config":
        options.configPath = requireValue(argv, ++idx, arg);
        break;
      case "--first-message":
        options.firstMessage = requireValue(argv, ++idx, arg);
        break;
      case "--second-message":
        options.secondMessage = requireValue(argv, ++idx, arg);
        break;
      case "--session-key":
        options.sessionKey = requireValue(argv, ++idx, arg);
        break;
      case "--timeout-ms":
        options.timeoutMs = Number.parseInt(requireValue(argv, ++idx, arg), 10);
        if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
          throw new Error("--timeout-ms must be a positive integer");
        }
        break;
      case "--help":
      case "-h":
        printHelp();
        process.exit(0);
      default:
        throw new Error(`unknown argument: ${arg}`);
    }
  }

  return options;
}

function requireValue(argv, idx, flag) {
  const value = argv[idx];
  if (!value) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
}

function printHelp() {
  console.log(`Usage: node scripts/openclaw-agent-smoke.mjs [options]

Options:
  --config <path>          OpenClaw config path
  --session-key <key>      OpenClaw agent session key
  --first-message <text>   First simulated inbound message
  --second-message <text>  Second simulated inbound message
  --timeout-ms <ms>        Gateway request timeout
`);
}

function expandHome(input) {
  if (input === "~") {
    return homedir();
  }
  if (input.startsWith("~/")) {
    return resolve(homedir(), input.slice(2));
  }
  return input;
}

async function readJson(path) {
  return JSON.parse(await readFile(expandHome(path), "utf8"));
}

class GatewayConnection {
  constructor({ url, token, timeoutMs }) {
    this.nextId = 1;
    this.pending = new Map();
    this.timeoutMs = timeoutMs;
    this.socket = new WebSocket(url);
    this.connected = this.connect(token);
  }

  connect(token) {
    return new Promise((resolveConnect, rejectConnect) => {
      const connectTimeout = setTimeout(
        () => rejectConnect(new Error("timed out waiting for gateway connect")),
        this.timeoutMs,
      );

      const failConnect = (error) => {
        clearTimeout(connectTimeout);
        rejectConnect(error instanceof Error ? error : new Error(String(error)));
      };

      this.socket.addEventListener("error", failConnect, { once: true });
      this.socket.addEventListener("message", (event) => {
        const frame = JSON.parse(event.data);
        if (frame.event === "connect.challenge") {
          this.socket.send(
            JSON.stringify({
              type: "req",
              id: "connect",
              method: "connect",
              params: {
                minProtocol: 4,
                maxProtocol: 4,
                client: {
                  id: "gateway-client",
                  displayName: "wecode smoke",
                  version: "0.1.0",
                  platform: process.platform,
                  mode: "backend",
                },
                caps: [],
                auth: { token },
                role: "operator",
                scopes: ["operator.read", "operator.write"],
              },
            }),
          );
          return;
        }

        if (frame.type === "res" && frame.id === "connect") {
          clearTimeout(connectTimeout);
          if (frame.ok) {
            resolveConnect(frame.payload);
          } else {
            rejectConnect(new Error(frame.error?.message || "gateway connect failed"));
          }
          return;
        }

        this.resolveResponse(frame);
      });
    });
  }

  async request(method, params) {
    await this.connected;

    const id = `req-${this.nextId}`;
    this.nextId += 1;

    const result = new Promise((resolveRequest, rejectRequest) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        rejectRequest(new Error(`gateway request timed out: ${method}`));
      }, this.timeoutMs);

      this.pending.set(id, {
        resolve: resolveRequest,
        reject: rejectRequest,
        timeout,
      });
    });

    this.socket.send(JSON.stringify({ type: "req", id, method, params }));
    return result;
  }

  resolveResponse(frame) {
    if (frame.type !== "res") {
      return;
    }

    const pending = this.pending.get(frame.id);
    if (!pending) {
      return;
    }

    if (frame.payload?.status === "accepted") {
      return;
    }

    clearTimeout(pending.timeout);
    this.pending.delete(frame.id);

    if (frame.ok) {
      pending.resolve(frame.payload);
    } else {
      pending.reject(new Error(frame.error?.message || "gateway request failed"));
    }
  }

  close() {
    this.socket.close();
  }
}

function agentParams({ message, sessionKey, timeoutMs }) {
  return {
    message,
    agentId: "main",
    sessionKey,
    deliver: false,
    timeout: Math.ceil(timeoutMs / 1000),
    idempotencyKey: randomUUID(),
  };
}

function extractText(result) {
  return result?.result?.payloads?.[0]?.text || "";
}

function extractCliSessionId(result) {
  return result?.result?.meta?.agentMeta?.cliSessionBinding?.sessionId || "";
}

async function run() {
  const options = parseArgs(process.argv.slice(2));
  const config = await readJson(options.configPath);
  const port = config.gateway?.port;
  const token = config.gateway?.auth?.token;

  if (!port || !token) {
    throw new Error("OpenClaw config must contain gateway.port and gateway.auth.token");
  }

  const gateway = new GatewayConnection({
    url: `ws://127.0.0.1:${port}`,
    token,
    timeoutMs: options.timeoutMs,
  });

  try {
    await gateway.connected;
    const first = await gateway.request(
      "agent",
      agentParams({
        message: options.firstMessage,
        sessionKey: options.sessionKey,
        timeoutMs: options.timeoutMs,
      }),
    );
    const second = await gateway.request(
      "agent",
      agentParams({
        message: options.secondMessage,
        sessionKey: options.sessionKey,
        timeoutMs: options.timeoutMs,
      }),
    );

    const firstText = extractText(first);
    const secondText = extractText(second);
    const firstCliSessionId = extractCliSessionId(first);
    const secondCliSessionId = extractCliSessionId(second);
    const resumeVerified =
      firstCliSessionId.length > 0 && firstCliSessionId === secondCliSessionId;

    console.log(`sessionKey: ${options.sessionKey}`);
    console.log(`firstReply: ${firstText}`);
    console.log(`secondReply: ${secondText}`);
    console.log(`cliSessionId: ${secondCliSessionId || "(missing)"}`);
    console.log(`resumeVerified: ${resumeVerified}`);

    if (
      firstText !== expectedReply(options.firstMessage) ||
      secondText !== expectedReply(options.secondMessage) ||
      !resumeVerified
    ) {
      process.exitCode = 1;
    }
  } finally {
    gateway.close();
  }
}

function expectedReply(message) {
  return message.replace(/^Reply exactly:\s*/, "").trim();
}

run().catch((error) => {
  console.error(`error: ${error.message}`);
  process.exit(1);
});
