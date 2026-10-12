# WESL Playground

Write, compile and preview [WESL](https://wesl-lang.dev) shaders with Claude. Ask for a shader
and Claude writes it, compiles it, reads the compiler errors, fixes them and shows the result
in a fly-through 3D viewer built on Bevy (WebGPU or WebGL2) that runs in your browser.
Related variants go into a group and can be shown side by side, and shared modules let
variants reuse the same code.

The plugin adds an MCP server with the playground's tools and a skill that teaches Claude
how to use them: read the authoring guide first, compile until there are no errors, then
check the result in the viewer.

## Requirements

- **Node.js 18 or newer** on your `PATH`. The plugin's server is a single Node script with no
  dependencies; nothing is downloaded or installed.
- A browser with WebGPU or WebGL2 (any recent Chrome, Edge, Firefox or Safari).

## Usage

1. Install the plugin in Claude Code:

   ```
   /plugin install wesl-playground --marketplace vovac12/wesl-playground
   ```

   On older Claude Code versions, run `/plugin marketplace add vovac12/wesl-playground` and
   then `/plugin install wesl-playground@wesl-playground`.
2. Open the viewer at **https://vovac12.github.io/wesl-playground/** and keep the tab
   visible. Browsers pause hidden tabs, so tool calls time out if the tab is in the
   background. The page connects to the plugin by itself; Chrome may ask once to let it
   access devices on your local network.
3. Ask for some shaders.

You can fly around in the viewer: hold the right mouse button to look, move with W A S D,
go down and up with Q and E, and hold Shift to go faster. Press Tab for the project menu.

### Example prompts

- *"Make 3 portal shader variants and show them side by side."*
- *"Create a lava lamp blob with vertex displacement and a noise texture, then open it."*
- *"The rings example has a compile error; fix it and take a screenshot."*

## Tools

| Tool                  | What it does                                                         |
| --------------------- | -------------------------------------------------------------------- |
| `shader_guide`        | Returns the authoring guide: manifest format, WESL rules, templates   |
| `list_projects`       | Lists groups and projects                                            |
| `get_project`         | Returns a project's manifest, file list and sources                  |
| `create_project`      | Creates a project, compiles it and returns errors                    |
| `update_project`      | Writes or deletes files or replaces the manifest, then recompiles    |
| `delete_project`      | Permanently deletes a project and its files                          |
| `set_group`           | Sets a group's display name, description and spacing                 |
| `update_shared_files` | Writes WESL modules shared by a group and recompiles the group       |
| `add_texture`         | Adds a procedural, base64 or local image texture to a project        |
| `import_file`         | Copies a binary file (such as a .glb mesh) into a project            |
| `compile_project`     | Compiles off-screen and returns errors and an optional preview       |
| `open_in_viewer`      | Shows a project or a whole group in the viewer                       |
| `screenshot`          | Captures what the viewer currently shows                             |
| `viewer_state`        | Reports what the viewer shows and its recent errors and warnings    |

The skill `wesl-shaders` tells Claude to call `shader_guide` before writing shaders,
recompile until a project is clean, use groups for variants, and check results with
`open_in_viewer` and `screenshot`.

## What this plugin runs and connects to

- **A local Node process.** Claude Code starts `server/bridge.mjs` with Node and talks to it
  over stdio (standard input and output). It is plain, readable JavaScript with no
  dependencies.
- **A local WebSocket server.** The bridge listens on `127.0.0.1:7880` (localhost only; the
  port can be changed). It accepts WebSocket connections only from pages on the viewer's
  origin, `https://vovac12.github.io`, plus any origins you add yourself. Connections
  without an Origin header, which come from local programs rather than browsers, are refused
  unless you opt in. Other HTTP requests on that port get a one-line text reply.
- **Your project files.** Projects are stored under `~/.wesl-playground/projects` (or the
  folder in `WESL_PLAYGROUND_DIR`), which the bridge creates on start. The bridge reads and
  writes files only inside that folder. When the viewer connects, the bridge sends it a copy
  of the folder's files, and the viewer writes changes back. A viewer used without the
  bridge keeps projects in the browser, and these are copied into the folder the next time
  it connects. `delete_project` removes a project's files permanently; they do not go to the
  trash.
- **Other local files only on request.** `add_texture` and `import_file` read a file
  elsewhere on your disk only when a tool call names it with `source_path`, and only if its
  extension is one the bridge allows: .png, .jpg, .jpeg, .ktx2 or .hdr for `add_texture`, and
  those plus .glb, .gltf or .bin for `import_file`. The limit is 64 MB per file.
- **The viewer comes from GitHub Pages.** The viewer is a WebAssembly app served from
  https://vovac12.github.io/wesl-playground/. It is not bundled in the plugin. The bridge
  itself makes no outgoing network connections. The viewer's source is in the repository
  under `src/` and `web/`.
- **No telemetry, analytics, accounts or credentials.** Nothing is collected or sent anywhere.
  See the [privacy policy](https://github.com/vovac12/wesl-playground/blob/main/PRIVACY.md).

## Configuration

Set these environment variables before starting Claude Code to change the defaults.

| Variable                          | Default                                      | Purpose                                                       |
| --------------------------------- | -------------------------------------------- | ------------------------------------------------------------- |
| `WESL_PLAYGROUND_DIR`             | `~/.wesl-playground/projects`                | Project folder                                                |
| `WESL_PLAYGROUND_PORT`            | `7880`                                       | WebSocket port for the browser viewer                         |
| `WESL_PLAYGROUND_URL`             | `https://vovac12.github.io/wesl-playground/` | Viewer the bridge points to                                   |
| `WESL_PLAYGROUND_ORIGINS`         | (none)                                       | Extra allowed page origins, comma separated (e.g. a local build) |
| `WESL_PLAYGROUND_ALLOW_NO_ORIGIN` | (off)                                        | Set to `1` to accept WebSocket clients that send no Origin (scripts, tests) |

If you change the port, open the viewer with a matching `?bridge=ws://127.0.0.1:PORT` added to
its URL.

## Troubleshooting

- **"No WESL Playground viewer is running"**: open https://vovac12.github.io/wesl-playground/
  and keep the tab open, then ask Claude to retry.
- **Tool calls time out**: the viewer tab is hidden or in the background, and the browser has
  paused it. Bring the tab to the front.
- **Port 7880 is in use**: another bridge is already running, for example a second Claude Code
  session or the Claude Desktop extension. Close it, or set `WESL_PLAYGROUND_PORT` and open the
  viewer with `?bridge=ws://127.0.0.1:PORT`.
- **The page can't reach the bridge**: allow the page to access your local network when Chrome
  asks, and check that the plugin's server is running with `/mcp` in Claude Code.
- **Logs**: run `claude --debug` to see the bridge's messages.

## More

The [repository](https://github.com/vovac12/wesl-playground) has the full documentation: the
project format, the native app with its own MCP server, the Claude Desktop extension, and how
to build the viewer yourself. Report issues at
https://github.com/vovac12/wesl-playground/issues.

License: MIT.
