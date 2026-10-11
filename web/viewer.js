// Shared by the standalone page (index.html) and the Claude Desktop inline view (app.js):
// starts the WASM viewer with a storage backend and talks the bridge's viewer protocol.
//
// Viewer protocol (JSON-RPC, over a WebSocket or MCP Apps tool calls):
//   viewer -> bridge  request       storage/snapshot              -> { files: { path: base64 } }
//   viewer -> bridge  notification  storage/put { path, data }    (data is base64)
//   viewer -> bridge  notification  storage/remove { path }
//   bridge -> viewer  request       tools/call { name, arguments } -> MCP tool result
//   bridge -> viewer  notification  viewer/superseded             another viewer took over

export function toBase64(bytes) {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

export function fromBase64(text) {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** Converts a `storage/snapshot` result into the viewer's initial files. */
export function snapshotEntries(files) {
  return Object.entries(files).map(([path, data]) => [path, fromBase64(data)]);
}

/**
 * Starts the WASM app. `storage` provides `initialFiles` ([path, Uint8Array][]) and
 * `put(path, bytes)` / `remove(path)`, called for every write the app makes.
 * Resolves to the app's exported API once it is running.
 */
export async function startViewer(storage) {
  window.shadertoyStorage = {
    initialFiles: () => storage.initialFiles,
    // `data` is a view into WASM memory; hand out a copy.
    put: (path, data) => storage.put(path, data.slice()),
    remove: (path) => storage.remove(path),
  };
  const wasm = await import(new URL("./shadertoy.js", import.meta.url));
  try {
    await wasm.default();
  } catch (e) {
    // winit uses an exception to hand control to the browser's event loop.
    if (!String(e).includes("Using exceptions for control flow")) throw e;
  }
  return {
    /** Handles a JSON-RPC message string; resolves to the response string or undefined. */
    handle: (message) => wasm.mcp_handle(message),
    apply: (path, bytes) => wasm.storage_apply(path, bytes ?? undefined),
    shutdown: () => wasm.shutdown(),
    setLabel: (text) => wasm.set_connection_label(text),
  };
}

/**
 * Dispatches a message from the bridge. `send(message)` delivers a reply.
 * Returns "superseded" when another viewer took over.
 */
export async function handleBridgeMessage(app, message, send) {
  if (message.method === "viewer/superseded") return "superseded";
  if (message.method === "tools/call" && message.id !== undefined) {
    const response = await app.handle(JSON.stringify(message));
    if (response) send(JSON.parse(response));
  }
}

/** Applies a fresh snapshot: changed or new files, and removals of files no longer present. */
export function applySnapshot(app, known, files) {
  for (const [path, data] of Object.entries(files)) {
    if (known.get(path) !== data) {
      app.apply(path, fromBase64(data));
      known.set(path, data);
    }
  }
  for (const path of [...known.keys()]) {
    if (!(path in files)) {
      app.apply(path, null);
      known.delete(path);
    }
  }
}
