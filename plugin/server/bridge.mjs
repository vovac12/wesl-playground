#!/usr/bin/env node
// Stdio MCP server for the WESL Playground. It owns the project folder and relays tool calls to
// a viewer, which compiles and renders the shaders:
//
//   Claude ──stdio── bridge ──WebSocket── browser tab (GitHub Pages, ws://127.0.0.1:7880)
//                       └──── MCP Apps ─── inline viewer in Claude Desktop (via the host)
//
// The bridge answers `initialize` and `tools/list` itself, so the tools exist before any viewer
// runs. Only one viewer is active at a time; the newest one takes over. No dependencies.
//
// Environment:
//   WESL_PLAYGROUND_DIR      project folder (default ~/.wesl-playground/projects)
//   WESL_PLAYGROUND_PORT     WebSocket port for browser tabs (default 7880)
//   WESL_PLAYGROUND_URL      viewer URL (default https://vovac12.github.io/wesl-playground/)
//   WESL_PLAYGROUND_ORIGINS  extra allowed page origins, comma separated

import { createHash, randomUUID } from "node:crypto";
import { mkdirSync, readdirSync, readFileSync, rmdirSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { homedir } from "node:os";
import { dirname, join, resolve, sep } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const TOOLS = JSON.parse(readFileSync(join(here, "tools.json"), "utf8"));
const GUIDE = readFileSync(join(here, "guide.md"), "utf8");
const VERSION = JSON.parse(readFileSync(join(here, "..", ".claude-plugin", "plugin.json"), "utf8")).version;

const PORT = Number(process.env.WESL_PLAYGROUND_PORT || 7880);
const VIEWER_URL = (process.env.WESL_PLAYGROUND_URL || "https://vovac12.github.io/wesl-playground/").replace(/\/?$/, "/");
const VIEWER_ORIGIN = new URL(VIEWER_URL).origin;
const PROJECTS = resolve(process.env.WESL_PLAYGROUND_DIR || join(homedir(), ".wesl-playground", "projects"));
const CALL_TIMEOUT_MS = 90_000;
const BOOT_WAIT_MS = 60_000; // how long tools wait for an inline viewer that is loading
const POLL_HOLD_MS = 20_000; // long-poll duration for inline viewers
const APP_VIEWER_TTL_MS = 45_000; // an inline viewer that stopped polling is gone

const UI_EXTENSION = "io.modelcontextprotocol/ui";
const UI_MIME = "text/html;profile=mcp-app";
const VIEW_URI = "ui://wesl-playground/viewer";

const log = (...args) => console.error("[wesl-playground]", ...args);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------------------------------------
// Project storage (the folder is the source of truth; viewers keep a mirror)

mkdirSync(PROJECTS, { recursive: true });

function storagePath(rel) {
  if (typeof rel !== "string" || !rel || rel.includes("\\") || rel.split("/").some((p) => !p || p === "." || p === "..")) {
    throw new Error(`invalid path ${JSON.stringify(rel)}`);
  }
  const full = resolve(PROJECTS, rel);
  if (!full.startsWith(PROJECTS + sep)) throw new Error(`path escapes the project folder: ${rel}`);
  return full;
}

function snapshot() {
  const files = {};
  const walk = (dir, prefix) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.name.startsWith(".")) continue;
      const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) walk(join(dir, entry.name), rel);
      else files[rel] = readFileSync(join(dir, entry.name)).toString("base64");
    }
  };
  walk(PROJECTS, "");
  return files;
}

function storagePut(rel, base64) {
  const full = storagePath(rel);
  mkdirSync(dirname(full), { recursive: true });
  writeFileSync(full, Buffer.from(base64, "base64"));
}

function storageRemove(rel) {
  const full = storagePath(rel);
  try {
    unlinkSync(full);
  } catch {
    return;
  }
  // Drop directories that became empty.
  for (let dir = dirname(full); dir.startsWith(PROJECTS + sep); dir = dirname(dir)) {
    try {
      if (readdirSync(dir).length) break;
      rmdirSync(dir);
    } catch {
      break;
    }
  }
}

// ---------------------------------------------------------------------------------------------
// Viewers

/** @type {{ id: string, kind: "ws"|"app", send(message: object): void, close(): void, alive(): boolean } | null} */
let viewer = null;
let bootingUntil = 0; // an inline viewer was requested and is probably loading
const viewerWaiters = new Set();

function activate(next) {
  const previous = viewer;
  viewer = next;
  if (previous) {
    failPending(previous, "The viewer was replaced by a newer one; please retry.");
    previous.send({ jsonrpc: "2.0", method: "viewer/superseded" });
    previous.close();
  }
  bootingUntil = 0;
  for (const wake of viewerWaiters) wake();
  log(`viewer ${next.kind} ${next.id} active`);
}

function deactivate(v, reason) {
  if (viewer !== v) return;
  viewer = null;
  failPending(v, reason);
  log(`viewer ${v.id} gone: ${reason}`);
}

async function waitForViewer() {
  if (viewer?.alive()) return viewer;
  if (viewer) deactivate(viewer, "The viewer stopped responding.");
  while (Date.now() < bootingUntil) {
    await new Promise((resolve) => {
      const wake = () => { viewerWaiters.delete(wake); clearTimeout(timer); resolve(); };
      const timer = setTimeout(wake, Math.max(0, bootingUntil - Date.now()));
      viewerWaiters.add(wake);
    });
    if (viewer?.alive()) return viewer;
  }
  return null;
}

// Tool calls forwarded to the viewer, by bridge-generated id.
let nextId = 1;
const pending = new Map(); // id -> { viewer, resolve, timer }

function forward(v, params) {
  return new Promise((resolve) => {
    const id = `bridge-${nextId++}`;
    const timer = setTimeout(() => {
      pending.delete(id);
      resolve(errorResult(`The viewer did not answer within ${CALL_TIMEOUT_MS / 1000}s (is it hidden, frozen or in a background tab?).`));
    }, CALL_TIMEOUT_MS);
    pending.set(id, { viewer: v, resolve, timer });
    v.send({ jsonrpc: "2.0", id, method: "tools/call", params });
  });
}

function failPending(v, reason) {
  for (const [id, entry] of pending) {
    if (entry.viewer !== v) continue;
    clearTimeout(entry.timer);
    pending.delete(id);
    entry.resolve(errorResult(reason));
  }
}

/** Handles a message from a viewer; returns a response for requests it answers. */
function fromViewer(v, message) {
  if (message.method === "bridge/info") {
    return { jsonrpc: "2.0", id: message.id, result: { version: VERSION, protocol: 1 } };
  }
  if (message.method === "storage/snapshot") {
    // A tab claims the viewer role when it starts (or resumes) in bridge mode.
    if (v.kind === "ws" && viewer !== v) activate(v);
    return { jsonrpc: "2.0", id: message.id, result: { files: snapshot() } };
  }
  if (viewer !== v) return; // superseded viewers may not write
  try {
    if (message.method === "storage/put") return storagePut(message.params.path, message.params.data);
    if (message.method === "storage/remove") return storageRemove(message.params.path);
  } catch (e) {
    return log(`storage: ${e.message}`);
  }
  const entry = pending.get(message.id);
  if (entry && entry.viewer === v) {
    clearTimeout(entry.timer);
    pending.delete(message.id);
    entry.resolve(message.error ? errorResult(message.error.message) : message.result);
  }
}

// ---- Inline viewers (MCP Apps): reached through app-only tools proxied by the host.

const appViewers = new Map(); // viewerId -> { viewer, queue, waiter, lastSeen }

function appTool(name, args) {
  if (name === "viewer_connect") {
    // A view rendered for an old tool call (e.g. when a conversation is reopened) must not steal
    // the viewer; it waits for the user to click instead.
    const fresh = Date.now() < bootingUntil || !viewer?.alive();
    if (!fresh && !args.force) return { needsActivation: true };
    const state = { queue: [], waiter: null, lastSeen: Date.now(), closed: false };
    const v = {
      id: randomUUID(),
      kind: "app",
      send(message) {
        state.queue.push(message);
        state.waiter?.();
      },
      close() {
        state.closed = true;
        state.waiter?.();
      },
      alive: () => !state.closed && Date.now() - state.lastSeen < APP_VIEWER_TTL_MS,
    };
    state.viewer = v;
    appViewers.set(v.id, state);
    activate(v);
    return { viewerId: v.id, files: snapshot() };
  }

  const state = appViewers.get(args.viewerId);
  if (!state) return { superseded: true };
  state.lastSeen = Date.now();

  if (name === "viewer_send") {
    for (const message of args.messages ?? []) {
      const reply = fromViewer(state.viewer, message);
      if (reply) state.queue.push(reply);
    }
    return { ok: true };
  }

  if (name === "viewer_poll") {
    return (async () => {
      if (!state.queue.length && !state.closed) {
        await new Promise((resolve) => {
          const timer = setTimeout(() => { state.waiter = null; resolve(); }, POLL_HOLD_MS);
          state.waiter = () => { clearTimeout(timer); state.waiter = null; resolve(); };
        });
      }
      state.lastSeen = Date.now();
      const messages = state.queue.splice(0);
      if (state.closed) appViewers.delete(args.viewerId);
      return { messages, superseded: state.closed && !messages.length };
    })();
  }
  throw new Error(`unknown app tool ${name}`);
}

const APP_TOOLS = [
  {
    name: "viewer_connect",
    description: "Inline viewer: attach to the bridge and fetch the project files.",
    inputSchema: { type: "object", properties: { force: { type: "boolean" } } },
  },
  {
    name: "viewer_poll",
    description: "Inline viewer: wait for requests from the bridge.",
    inputSchema: { type: "object", properties: { viewerId: { type: "string" } }, required: ["viewerId"] },
  },
  {
    name: "viewer_send",
    description: "Inline viewer: send responses and storage changes to the bridge.",
    inputSchema: {
      type: "object",
      properties: { viewerId: { type: "string" }, messages: { type: "array" } },
      required: ["viewerId", "messages"],
    },
  },
].map((tool) => ({ ...tool, _meta: { ui: { resourceUri: VIEW_URI, visibility: ["app"] } } }));

const VIEW_HTML = `<!doctype html>
<html><head><meta charset="utf-8"><style>
html,body{margin:0;height:100%;overflow:hidden;background:#1e1f24;color:#c9cbd3;font:14px/1.5 system-ui,sans-serif}
canvas{display:block;width:100%;height:100%;outline:none}
#overlay{position:fixed;inset:0;display:grid;place-items:center;text-align:center;padding:16px;background:#1e1f24}
#fullscreen{position:fixed;top:8px;right:8px;display:none;padding:4px 10px;border-radius:6px;border:0;background:#2c2e36;color:#ddd;cursor:pointer}
button{font:inherit}
</style></head><body>
<canvas id="bevy" tabindex="0"></canvas>
<div id="overlay">Loading WESL Playground…</div>
<button id="fullscreen">Fullscreen</button>
<script type="module">
import(${JSON.stringify(VIEWER_URL + "app.js")}).catch((e) => {
  document.getElementById("overlay").textContent = "Could not load the viewer from ${VIEWER_URL}: " + e;
});
</script></body></html>`;

// ---------------------------------------------------------------------------------------------
// Claude side (stdio, newline-delimited JSON-RPC)

let clientSupportsUi = false;

const toClaude = (message) => process.stdout.write(JSON.stringify(message) + "\n");
const textResult = (text) => ({ content: [{ type: "text", text }] });
const errorResult = (text) => ({ content: [{ type: "text", text }], isError: true });

function noViewerText() {
  const fix = clientSupportsUi
    ? "Call `open_in_viewer` (with any project or group) to start the inline viewer, then retry."
    : `Ask the user to open ${VIEWER_URL} in a browser and keep the tab open; it connects to ws://127.0.0.1:${PORT} automatically.`;
  return `No WESL Playground viewer is running. ${fix}` + (listenError ? `\nBridge problem: ${listenError}` : "");
}

function toolList() {
  if (!clientSupportsUi) return TOOLS;
  const tools = TOOLS.map((tool) =>
    tool.name === "open_in_viewer"
      ? {
          ...tool,
          description:
            "Show a project, or a whole group side by side, in an interactive 3D viewer embedded in the chat. " +
            "Also starts the viewer that the other tools need; call it before creating or compiling projects.",
          _meta: { ui: { resourceUri: VIEW_URI } },
        }
      : tool,
  );
  return [...tools, ...APP_TOOLS];
}

async function callTool(params) {
  const name = params.name;
  const args = { ...(params.arguments ?? {}) };
  if (name === "shader_guide") return textResult(GUIDE);
  if (APP_TOOLS.some((t) => t.name === name)) {
    const result = await appTool(name, args);
    return { content: [{ type: "text", text: JSON.stringify({ ...result, files: undefined }) }], structuredContent: result };
  }

  // Inline viewer: the host renders it after this call returns, so answer right away.
  if (name === "open_in_viewer" && clientSupportsUi) {
    bootingUntil = Date.now() + BOOT_WAIT_MS;
    const what = args.project ? `project \`${args.project}\`` : args.group ? `group \`${args.group}\`` : "the playground";
    return textResult(
      `Opening ${what} in the inline viewer below. It takes a few seconds to load the first time; ` +
        "other tools wait for it. The user can fly around: right mouse to look, WASD/QE to move.",
    );
  }

  // The viewer cannot read local files, so inline them.
  if (typeof args.source_path === "string") {
    try {
      args.base64 = readFileSync(args.source_path).toString("base64");
      delete args.source_path;
    } catch (e) {
      return errorResult(`${args.source_path}: ${e.message}`);
    }
  }
  const v = await waitForViewer();
  if (!v) return errorResult(noViewerText());
  return forward(v, { ...params, arguments: args });
}

async function handleClaude(message) {
  const { id, method, params } = message;
  if (id === undefined) return; // notification
  const reply = (result) => toClaude({ jsonrpc: "2.0", id, result });
  switch (method) {
    case "initialize":
      clientSupportsUi = !!params?.capabilities?.extensions?.[UI_EXTENSION]?.mimeTypes?.includes(UI_MIME);
      log(`client ${params?.clientInfo?.name ?? "?"}, MCP Apps: ${clientSupportsUi}`);
      return reply({
        protocolVersion: params?.protocolVersion ?? "2025-06-18",
        capabilities: { tools: { listChanged: false }, ...(clientSupportsUi ? { resources: {} } : {}) },
        serverInfo: { name: "wesl-playground", version: VERSION },
        instructions:
          `${GUIDE}\n\n## Viewer\n` +
          (clientSupportsUi
            ? "This client shows the viewer inline: call `open_in_viewer` first (it starts the viewer), then create and compile projects. "
            : `The viewer runs in the user's browser at ${VIEWER_URL}. If a tool reports that no viewer is running, ask the user to open that page. `) +
          `Projects are stored in ${PROJECTS}.`,
      });
    case "ping":
      return reply({});
    case "tools/list":
      return reply({ tools: toolList() });
    case "resources/list":
      return reply({
        resources: clientSupportsUi ? [{ uri: VIEW_URI, name: "WESL Playground viewer", mimeType: UI_MIME }] : [],
      });
    case "resources/read":
      if (params?.uri !== VIEW_URI) {
        return toClaude({ jsonrpc: "2.0", id, error: { code: -32002, message: `unknown resource ${params?.uri}` } });
      }
      return reply({
        contents: [{
          uri: VIEW_URI,
          mimeType: UI_MIME,
          text: VIEW_HTML,
          _meta: { ui: { csp: { connectDomains: [VIEWER_ORIGIN], resourceDomains: [VIEWER_ORIGIN] }, prefersBorder: false } },
        }],
      });
    case "tools/call":
      try {
        return reply(await callTool(params ?? {}));
      } catch (e) {
        return reply(errorResult(e.message));
      }
    default:
      return toClaude({ jsonrpc: "2.0", id, error: { code: -32601, message: `method not found: ${method}` } });
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
  // Long-polls must not block other messages, so every message is handled concurrently.
  for (const m of Array.isArray(message) ? message : [message]) handleClaude(m);
});
process.stdin.on("end", () => process.exit(0));

// ---------------------------------------------------------------------------------------------
// Browser tabs (minimal RFC 6455 WebSocket server: text frames, ping/pong, close)

let listenError = null;

// Only the viewer may connect: any other site could otherwise pose as the viewer.
const allowedOrigins = [
  VIEWER_ORIGIN,
  ...(process.env.WESL_PLAYGROUND_ORIGINS || "").split(",").map((s) => s.trim()).filter(Boolean),
];
const isAllowedOrigin = (origin) =>
  !origin || // non-browser clients (tests, curl) send no Origin
  allowedOrigins.includes(origin) ||
  /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\])(:\d+)?$/.test(origin);

class Socket {
  constructor(socket, onText) {
    this.socket = socket;
    this.onText = onText;
    this.buffer = Buffer.alloc(0);
    this.fragments = [];
    this.open = true;
    socket.on("data", (chunk) => this.receive(chunk));
  }

  send(text, opcode = 0x1) {
    if (!this.open) return;
    const payload = Buffer.from(text);
    const len = payload.length;
    const header =
      len < 126 ? Buffer.from([0x80 | opcode, len])
      : len < 65536 ? Buffer.from([0x80 | opcode, 126, len >> 8, len & 0xff])
      : Buffer.concat([Buffer.from([0x80 | opcode, 127]), bigLength(len)]);
    this.socket.write(Buffer.concat([header, payload]));
  }

  close() {
    if (!this.open) return;
    this.send("", 0x8);
    this.open = false;
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
          this.onText(text);
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

server.on("upgrade", (req, rawSocket) => {
  const origin = req.headers.origin;
  const key = req.headers["sec-websocket-key"];
  if (!key || !isAllowedOrigin(origin)) {
    log(`rejected connection from origin ${origin}`);
    rawSocket.end("HTTP/1.1 403 Forbidden\r\n\r\n");
    return;
  }
  const accept = createHash("sha1").update(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest("base64");
  rawSocket.write(
    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
  );
  rawSocket.setNoDelay(true);

  const v = { id: randomUUID(), kind: "ws" };
  const socket = new Socket(rawSocket, (text) => {
    let message;
    try {
      message = JSON.parse(text);
    } catch {
      return log("invalid JSON from viewer");
    }
    const reply = fromViewer(v, message);
    if (reply) socket.send(JSON.stringify(reply));
  });
  Object.assign(v, {
    send: (message) => socket.send(JSON.stringify(message)),
    close: () => socket.close(),
    alive: () => socket.open,
  });
  const dropped = () => {
    socket.open = false;
    deactivate(v, "The viewer tab was closed or reloaded; please retry.");
  };
  rawSocket.on("close", dropped);
  rawSocket.on("error", dropped);
});

server.on("error", (e) => {
  listenError =
    e.code === "EADDRINUSE"
      ? `port ${PORT} is already in use (another Claude session may own the browser-tab connection)`
      : e.message;
  log(listenError);
});

server.listen(PORT, "127.0.0.1", () => log(`projects in ${PROJECTS}; waiting for a viewer on ws://127.0.0.1:${PORT}`));
