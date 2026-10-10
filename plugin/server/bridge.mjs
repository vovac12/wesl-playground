#!/usr/bin/env node
// Stdio MCP server that relays tool calls to the WESL Playground running in a browser tab.
//
// Claude Code <-stdio-> bridge <-WebSocket-> viewer page (GitHub Pages or local)
//
// The bridge answers `initialize` and `tools/list` itself (so Claude sees the tools before a
// tab is open) and forwards `tools/call` to the connected page. No dependencies.
//
// Environment:
//   WESL_PLAYGROUND_PORT     WebSocket port (default 7880)
//   WESL_PLAYGROUND_URL      viewer URL shown to the user
//   WESL_PLAYGROUND_ORIGINS  extra allowed page origins, comma separated

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const TOOLS = JSON.parse(readFileSync(join(here, "tools.json"), "utf8"));
const GUIDE = readFileSync(join(here, "guide.md"), "utf8");
const VERSION = JSON.parse(readFileSync(join(here, "..", ".claude-plugin", "plugin.json"), "utf8")).version;

const PORT = Number(process.env.WESL_PLAYGROUND_PORT || 7880);
const VIEWER_URL = process.env.WESL_PLAYGROUND_URL || "https://vovac12.github.io/wesl-playground/";
const CALL_TIMEOUT_MS = 90_000;

// Only the viewer may connect: any other site could otherwise pose as the viewer.
const allowedOrigins = [
  "https://vovac12.github.io",
  ...(process.env.WESL_PLAYGROUND_ORIGINS || "").split(",").map((s) => s.trim()).filter(Boolean),
];
const isAllowedOrigin = (origin) =>
  !origin || // non-browser clients (tests, curl) send no Origin
  allowedOrigins.includes(origin) ||
  /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\])(:\d+)?$/.test(origin);

const log = (...args) => console.error("[wesl-playground]", ...args);

// ---------------------------------------------------------------------------------------------
// Claude side (stdio, newline-delimited JSON-RPC)

const toClaude = (message) => process.stdout.write(JSON.stringify(message) + "\n");
const result = (id, value) => toClaude({ jsonrpc: "2.0", id, result: value });
const toolError = (id, text) =>
  result(id, { content: [{ type: "text", text }], isError: true });

const notConnectedText = () =>
  `No WESL Playground viewer is connected. Ask the user to open ${VIEWER_URL} ` +
  `in a browser (keep the tab open); it connects to ws://127.0.0.1:${PORT} automatically.` +
  (listenError ? `\nBridge problem: ${listenError}` : "");

function handleClaude(message) {
  const { id, method, params } = message;
  if (id === undefined) return; // notification
  switch (method) {
    case "initialize":
      return result(id, {
        protocolVersion: params?.protocolVersion ?? "2025-06-18",
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: "wesl-playground", version: VERSION },
        instructions:
          `${GUIDE}\n\n## Viewer\nThe viewer runs in the user's browser at ${VIEWER_URL}. ` +
          `If a tool reports that no viewer is connected, ask the user to open that page.`,
      });
    case "ping":
      return result(id, {});
    case "tools/list":
      return result(id, { tools: TOOLS });
    case "tools/call":
      return callTool(id, params ?? {});
    default:
      return toClaude({ jsonrpc: "2.0", id, error: { code: -32601, message: `method not found: ${method}` } });
  }
}

let nextId = 1;
const pending = new Map(); // bridge id -> { claudeId, timer }

function callTool(claudeId, params) {
  if (params.name === "shader_guide") {
    return result(claudeId, { content: [{ type: "text", text: GUIDE }] });
  }
  if (!page) return toolError(claudeId, notConnectedText());

  // The page cannot read local files, so inline them.
  const args = { ...(params.arguments ?? {}) };
  if (typeof args.source_path === "string") {
    try {
      args.base64 = readFileSync(args.source_path).toString("base64");
      delete args.source_path;
    } catch (e) {
      return toolError(claudeId, `${args.source_path}: ${e.message}`);
    }
  }

  const id = nextId++;
  const timer = setTimeout(() => {
    pending.delete(id);
    toolError(claudeId, `The viewer did not answer within ${CALL_TIMEOUT_MS / 1000}s (is the tab in the background or frozen?).`);
  }, CALL_TIMEOUT_MS);
  pending.set(id, { claudeId, timer });
  page.send(JSON.stringify({ jsonrpc: "2.0", id, method: "tools/call", params: { ...params, arguments: args } }));
}

function handlePage(text) {
  let message;
  try {
    message = JSON.parse(text);
  } catch {
    return log("invalid JSON from viewer");
  }
  const entry = pending.get(message.id);
  if (!entry) return;
  clearTimeout(entry.timer);
  pending.delete(message.id);
  toClaude({ ...message, id: entry.claudeId });
}

function failPending(reason) {
  for (const [id, { claudeId, timer }] of pending) {
    clearTimeout(timer);
    toolError(claudeId, reason);
    pending.delete(id);
  }
}

createInterface({ input: process.stdin }).on("line", (line) => {
  if (!line.trim()) return;
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return toClaude({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "parse error" } });
  }
  for (const m of Array.isArray(message) ? message : [message]) handleClaude(m);
});
process.stdin.on("end", () => process.exit(0));

// ---------------------------------------------------------------------------------------------
// Viewer side (minimal RFC 6455 WebSocket server: text frames, ping/pong, close)

let page = null;
let listenError = null;

class Socket {
  constructor(socket) {
    this.socket = socket;
    this.buffer = Buffer.alloc(0);
    this.fragments = [];
    socket.on("data", (chunk) => this.receive(chunk));
  }

  send(text, opcode = 0x1) {
    const payload = Buffer.from(text);
    const len = payload.length;
    const header =
      len < 126 ? Buffer.from([0x80 | opcode, len])
      : len < 65536 ? Buffer.from([0x80 | opcode, 126, len >> 8, len & 0xff])
      : Buffer.concat([Buffer.from([0x80 | opcode, 127]), bigLength(len)]);
    this.socket.write(Buffer.concat([header, payload]));
  }

  close() {
    try {
      this.send("", 0x8);
    } catch {}
    this.socket.end();
  }

  receive(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    for (;;) {
      const b = this.buffer;
      if (b.length < 2) return;
      const fin = b[0] & 0x80, opcode = b[0] & 0x0f, masked = b[1] & 0x80;
      let len = b[1] & 0x7f, offset = 2;
      if (len === 126) {
        if (b.length < 4) return;
        len = b.readUInt16BE(2);
        offset = 4;
      } else if (len === 127) {
        if (b.length < 10) return;
        len = Number(b.readBigUInt64BE(2));
        offset = 10;
      }
      const maskOffset = offset;
      if (masked) offset += 4;
      if (b.length < offset + len) return;
      const payload = Buffer.from(b.subarray(offset, offset + len));
      if (masked) for (let i = 0; i < len; i++) payload[i] ^= b[maskOffset + (i % 4)];
      this.buffer = b.subarray(offset + len);

      if (opcode === 0x8) return this.close();
      if (opcode === 0x9) { this.send(payload, 0xa); continue; }
      if (opcode === 0x1 || opcode === 0x0) {
        this.fragments.push(payload);
        if (fin) {
          const text = Buffer.concat(this.fragments).toString("utf8");
          this.fragments = [];
          if (this === page) handlePage(text);
        }
      }
    }
  }
}

function bigLength(len) {
  const b = Buffer.alloc(8);
  b.writeBigUInt64BE(BigInt(len));
  return b;
}

const server = createServer((_, res) => {
  res.writeHead(200, { "Content-Type": "text/plain" });
  res.end(`WESL Playground bridge. Open ${VIEWER_URL}\n`);
});

server.on("upgrade", (req, socket) => {
  const origin = req.headers.origin;
  const key = req.headers["sec-websocket-key"];
  if (!key || !isAllowedOrigin(origin)) {
    log(`rejected connection from origin ${origin}`);
    socket.end("HTTP/1.1 403 Forbidden\r\n\r\n");
    return;
  }
  const accept = createHash("sha1").update(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
  );
  socket.setNoDelay(true);

  // The newest tab wins.
  const previous = page;
  const current = new Socket(socket);
  page = current;
  if (previous) {
    failPending("The viewer was replaced by another tab; please retry.");
    previous.close();
  }
  log(`viewer connected (${origin ?? "no origin"})`);
  const dropped = () => {
    if (page !== current) return;
    page = null;
    failPending("The viewer tab was closed or reloaded; please retry.");
    log("viewer disconnected");
  };
  socket.on("close", dropped);
  socket.on("error", dropped);
});

server.on("error", (e) => {
  listenError =
    e.code === "EADDRINUSE"
      ? `port ${PORT} is already in use (another Claude Code session may own the viewer connection)`
      : e.message;
  log(listenError);
});

server.listen(PORT, "127.0.0.1", () => log(`waiting for the viewer on ws://127.0.0.1:${PORT}`));
