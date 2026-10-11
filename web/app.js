// MCP Apps view: the WASM viewer embedded inline in Claude Desktop.
//
// The host renders this view after an `open_in_viewer` tool call. The view reaches the bridge
// through app-only tools (`viewer_connect`, `viewer_poll`, `viewer_send`) proxied by the host,
// and then serves the model's tool calls (compile, screenshot, ...) like a browser tab would.

import { handleBridgeMessage, snapshotEntries, startViewer, toBase64 } from "./viewer.js";

const PROTOCOL_VERSION = "2026-01-26";
const DEFAULT_HEIGHT = 520;

// ---- Host messaging (JSON-RPC over postMessage).
let nextId = 0;
const pending = new Map();
const notificationHandlers = {};
window.addEventListener("message", (event) => {
  const message = event.data;
  if (!message || message.jsonrpc !== "2.0") return;
  if (message.id !== undefined && pending.has(message.id) && !message.method) {
    pending.get(message.id)(message);
    pending.delete(message.id);
  } else if (message.method === "ui/resource-teardown") {
    teardown();
    window.parent.postMessage({ jsonrpc: "2.0", id: message.id, result: {} }, "*");
  } else if (message.method && notificationHandlers[message.method]) {
    notificationHandlers[message.method](message.params ?? {});
  }
});
const request = (method, params) =>
  new Promise((resolve, reject) => {
    const id = `view-${++nextId}`;
    pending.set(id, (m) => (m.error ? reject(new Error(m.error.message)) : resolve(m.result)));
    window.parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*");
  });
const notify = (method, params) => window.parent.postMessage({ jsonrpc: "2.0", method, params }, "*");

/** Calls one of the bridge's app-only tools; returns its structured result. */
async function bridgeTool(name, args) {
  const result = await request("tools/call", { name, arguments: args });
  if (result.isError) throw new Error(result.content?.[0]?.text ?? `${name} failed`);
  return result.structuredContent ?? {};
}

// ---- Page chrome.
const overlay = document.getElementById("overlay");
const showOverlay = (html) => {
  overlay.innerHTML = html;
  overlay.style.display = html ? "grid" : "none";
};

function applySize(dimensions) {
  const fixedHeight = dimensions && "height" in dimensions;
  const height = fixedHeight ? dimensions.height : Math.min(DEFAULT_HEIGHT, dimensions?.maxHeight ?? DEFAULT_HEIGHT);
  document.documentElement.style.height = fixedHeight ? "100vh" : `${height}px`;
  if (!fixedHeight) notify("ui/notifications/size-changed", { width: document.body.clientWidth, height });
}

let displayMode = "inline";
let availableModes = [];
function addFullscreenButton() {
  if (!availableModes.includes("fullscreen")) return;
  const button = document.getElementById("fullscreen");
  button.style.display = "block";
  button.onclick = async () => {
    const mode = displayMode === "fullscreen" ? "inline" : "fullscreen";
    displayMode = (await request("ui/request-display-mode", { mode })).mode ?? displayMode;
    button.textContent = displayMode === "fullscreen" ? "Exit fullscreen" : "Fullscreen";
  };
}

// ---- Lifecycle.
let app = null;
let viewerId = null;
let active = false;
let toolArgs = null;
const toolArgsReady = new Promise((resolve) => {
  notificationHandlers["ui/notifications/tool-input"] = (params) => {
    toolArgs = params.arguments ?? {};
    resolve();
  };
});
notificationHandlers["ui/notifications/host-context-changed"] = (context) => {
  if (context.containerDimensions) applySize(context.containerDimensions);
  if (context.displayMode) displayMode = context.displayMode;
};

function teardown() {
  active = false;
  app?.shutdown();
}

const outbox = [];
let flushing = null;
async function flush() {
  if (flushing || !outbox.length || !viewerId) return;
  const messages = outbox.splice(0);
  flushing = bridgeTool("viewer_send", { viewerId, messages })
    .catch((e) => console.error("viewer_send failed", e))
    .finally(() => { flushing = null; if (outbox.length) flush(); });
}
const send = (message) => { outbox.push(message); flush(); };

async function pollLoop() {
  while (active) {
    let result;
    try {
      result = await bridgeTool("viewer_poll", { viewerId });
    } catch (e) {
      console.error("viewer_poll failed", e);
      await new Promise((r) => setTimeout(r, 2000));
      continue;
    }
    if (result.superseded) return superseded();
    for (const message of result.messages ?? []) {
      if ((await handleBridgeMessage(app, message, send)) === "superseded") return superseded();
    }
  }
}

function superseded() {
  active = false;
  app?.shutdown();
  document.getElementById("bevy").style.display = "none";
  showOverlay('<div>This viewer was replaced by a newer one.<br><button id="again">Reload this viewer</button></div>');
  document.getElementById("again").onclick = () => location.reload();
}

async function start(force) {
  showOverlay("<div>Connecting to the WESL Playground bridge…</div>");
  const connection = await bridgeTool("viewer_connect", { force });
  if (connection.needsActivation) {
    showOverlay('<div>This viewer is idle.<br><button id="use">Use this viewer</button></div>');
    document.getElementById("use").onclick = () => start(true);
    return;
  }
  viewerId = connection.viewerId;
  showOverlay("<div>Loading the 3D viewer…</div>");
  app = await startViewer({
    initialFiles: snapshotEntries(connection.files),
    put: (path, bytes) => send({ jsonrpc: "2.0", method: "storage/put", params: { path, data: toBase64(bytes) } }),
    remove: (path) => send({ jsonrpc: "2.0", method: "storage/remove", params: { path } }),
  });
  app.setLabel("Claude Desktop inline viewer: click to focus, right mouse to look, Tab for projects");
  showOverlay("");
  active = true;
  addFullscreenButton();
  // Show what the model asked for.
  await toolArgsReady;
  const { project, group } = toolArgs ?? {};
  if (project || group) {
    const args = project ? { project } : { group };
    await app.handle(JSON.stringify({ jsonrpc: "2.0", id: 0, method: "tools/call", params: { name: "open_in_viewer", arguments: args } }));
  }
  pollLoop();
}

try {
  const init = await request("ui/initialize", {
    protocolVersion: PROTOCOL_VERSION,
    appInfo: { name: "wesl-playground", version: "0.1.0" },
    appCapabilities: { availableDisplayModes: ["inline", "fullscreen"] },
  });
  notify("ui/notifications/initialized", {});
  const context = init.hostContext ?? {};
  availableModes = context.availableDisplayModes ?? [];
  displayMode = context.displayMode ?? "inline";
  applySize(context.containerDimensions);
  await start(false);
} catch (e) {
  showOverlay(`<div>Failed to start the viewer: ${String(e.message ?? e)}</div>`);
  throw e;
}
