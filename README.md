# Shader Playground

A 3D playground for [WESL](https://wesl-lang.dev) shaders, built on Bevy 0.20, driven by
[MCP](https://modelcontextprotocol.io). Ask Claude to write shaders, get compile errors back
automatically, then fly around the results: inline in Claude Desktop, in a browser tab, or in a
native app.

> *"Generate 5 variants of portal shaders"* → five projects in a `Portals` group, compiled
> and listed in the viewer. Open one, or show the whole group side by side.

## Features

- **Fly-through viewer**: free camera, a project dropdown, and a group view that lays out
  every project in the group with floating labels.
- **Runs where you are**: inline in the Claude Desktop chat (MCP Apps), in a browser tab
  (WebGPU/WebGL2 build on GitHub Pages), or as a native app.
- **Claude Code plugin and built-in MCP server**: agents create, edit, compile and delete projects,
  whether or not those projects are open in the viewer.
- **Real compile feedback**: projects compile in an off-screen sandbox, which reports WESL and
  naga errors, missing imports and asset failures, with an optional preview image.
- **Hot reload**: edits made by hand or through MCP show up immediately.
- **Batteries included**: material bindings, noise helpers, procedural textures, primitive,
  glTF and custom meshes.

## Quick start: Claude Desktop

Download **[wesl-playground.mcpb](https://github.com/vovac12/wesl-playground/releases/latest/download/wesl-playground.mcpb)**
and open it (or drag it into Settings → Extensions). Claude Desktop runs it with its built-in
Node.js; the extension settings let you pick the projects folder.

<details>
<summary>Manual setup from a clone (for development)</summary>

Needs Node.js 18+. Add the bridge to `claude_desktop_config.json` (Settings → Developer →
Edit Config) and restart Claude Desktop:

```json
{
  "mcpServers": {
    "wesl-playground": {
      "command": "node",
      "args": ["/path/to/wesl-playground/plugin/server/bridge.mjs"]
    }
  }
}
```

</details>

Ask for some shaders. Claude opens the viewer inline in the chat (`open_in_viewer`), then
creates and compiles projects through it. Click the viewer to focus it; it can also go
fullscreen.

### Example prompts

- *"Make 3 portal shader variants and show them side by side."*
- *"Create a lava lamp blob with vertex displacement and a noise texture, then open it."*
- *"Make the rings example spin faster and glow orange, then take a screenshot."*

## Quick start: Claude Code + browser

1. Install the Claude Code plugin (needs Node.js 18+):

   ```
   /plugin install wesl-playground --marketplace vovac12/wesl-playground
   ```

   On older Claude Code versions: `/plugin marketplace add vovac12/wesl-playground`, then
   `/plugin install wesl-playground@wesl-playground`.

2. Open the viewer: **https://vovac12.github.io/wesl-playground/** and keep the tab visible
   (browsers pause hidden tabs).
3. Run `claude` and ask for some shaders.

Chrome may ask to allow the page to access devices on your local network the first time.
See [`plugin/README.md`](plugin/README.md) for the plugin's tools, configuration, what it runs
and connects to, and troubleshooting.

### How the bridge works

Both setups run the same bridge (`plugin/server/bridge.mjs`), which Claude talks to over stdio.
It stores projects in a folder and relays tool calls to one viewer at a time: the inline view
in Claude Desktop, or a browser tab connected at `ws://127.0.0.1:7880`. The newest viewer takes
over. Without a bridge, the page still works on its own and keeps projects in the browser; they
are copied into the bridge's folder the next time it connects.

| Env var                   | Default                                       | Purpose                       |
| ------------------------- | --------------------------------------------- | ----------------------------- |
| `WESL_PLAYGROUND_DIR`     | `~/.wesl-playground/projects`                 | Project folder                |
| `WESL_PLAYGROUND_PORT`    | `7880`                                        | WebSocket port for tabs       |
| `WESL_PLAYGROUND_URL`     | `https://vovac12.github.io/wesl-playground/`  | Viewer the bridge points to   |
| `WESL_PLAYGROUND_ORIGINS` |                                               | Extra allowed page origins (e.g. a local build) |
| `WESL_PLAYGROUND_ALLOW_NO_ORIGIN` |                                       | `1` accepts WebSocket clients without an Origin (scripts, tests) |

Use `?bridge=ws://host:port` on the page URL to match a custom port. Point
`WESL_PLAYGROUND_DIR` at the native app's `BASE_FOLDER` to share projects with it.

## Quick start: native

Requires Rust 1.97+.

```sh
cargo run --release
```

Then run `claude` in this repo (it ships a `.mcp.json`) or register the server once:

```sh
claude mcp add --transport http wesl-playground-native http://127.0.0.1:7878/mcp
```

| Env var       | Default          | Purpose                     |
| ------------- | ---------------- | --------------------------- |
| `BASE_FOLDER` | `./projects`     | Where shader projects live  |
| `MCP_ADDR`    | `127.0.0.1:7878` | MCP server address (`/mcp`) |

On first launch both versions create an `examples` group.

### Building the web version

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129   # must match Cargo.lock
./web/build.sh                                     # output in dist/
```

Pushing to `main` deploys `dist/` to GitHub Pages (`.github/workflows/pages.yml`).

### Releasing

Bump the version in `Cargo.toml`, `plugin/.claude-plugin/plugin.json` and `plugin/manifest.json`,
then push a `vX.Y.Z` tag. `.github/workflows/release.yml` packs `plugin/` into
`wesl-playground.mcpb` and attaches it to a GitHub release. To pack locally:
`npx @anthropic-ai/mcpb pack plugin`.

## Controls

| Input              | Action                       |
| ------------------ | ---------------------------- |
| Right mouse (hold) | Look around                  |
| `W A S D` / `Q E`  | Move / down, up              |
| `Shift`, wheel     | Move faster, change speed    |
| `Tab`              | Open the project menu        |
| `R`                | Reload the current view      |

## Projects

```text
projects/
  portals/                 group (optional group.json)
    shared/                modules shared by the group, e.g. sdf.wesl
    swirl/
      project.json
      portal.wesl
  my_project/              ungrouped project
```

A project is a folder containing `project.json`. Folder names are `snake_case`, so files
can import each other with `import super::common::foo;`. Projects in a group import the
group's shared modules with `import super::super::shared::sdf::sd_torus;`, and editing a shared
module recompiles every project in the group.

```json
{
  "name": "Portal: swirl",
  "objects": [{
    "mesh": { "shape": "quad", "size": [2.5, 3.5] },
    "fragment_shader": "portal.wesl",
    "color": [0.3, 0.6, 1.0, 1.0],
    "alpha_mode": "blend",
    "double_sided": true,
    "position": [0, 1.75, 0]
  }]
}
```

```wgsl
import bevy_pbr::render::forward_io::VertexOutput;
import shadertoy::shaders::toy::{toy, time};
import shadertoy::shaders::noise::{fbm2, rotate2};

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.uv * 2.0 - 1.0;
    let r = length(p);
    let swirl = fbm2(rotate2(p, time() + r * 4.0) * 3.0);
    return vec4<f32>(toy.color.rgb * swirl * 2.0, smoothstep(1.0, 0.85, r));
}
```

See [`plugin/server/guide.md`](plugin/server/guide.md) for the full manifest format, bindings,
mesh shapes and a vertex shader template. Agents read it through the `shader_guide` tool.

## Troubleshooting

- **"No WESL Playground viewer is running"**: in Claude Desktop, ask Claude to open the viewer
  (`open_in_viewer`); in Claude Code, open https://vovac12.github.io/wesl-playground/ and keep
  the tab visible.
- **Calls time out**: hidden tabs and viewers scrolled out of view are paused by the browser.
  Keep the viewer visible, or use fullscreen in Claude Desktop.
- **Port 7880 is in use**: another bridge is running (e.g. Claude Code and Claude Desktop at
  once). The inline viewer still works; for tabs, set another port and add `?bridge=ws://127.0.0.1:PORT`.
- **"This viewer is idle"**: an older viewer in the chat was replaced; click it to use it again.
- Bridge logs: Claude Desktop → Settings → Developer, or `claude --debug` in Claude Code.

## MCP tools

| Tool                               | Description                                              |
| ---------------------------------- | -------------------------------------------------------- |
| `create_project`, `update_project` | Write manifest and files, compile, return errors/preview |
| `compile_project`                  | Compile off-screen, return errors and a preview image    |
| `list_projects`, `get_project`     | Browse groups, manifests and sources                     |
| `delete_project`, `set_group`      | Manage projects and group metadata                       |
| `update_shared_files`              | WESL modules shared by a group, recompiles its projects  |
| `add_texture`, `import_file`       | Procedural/base64/local textures, `.glb` meshes          |
| `open_in_viewer`                   | Show a project or group (inline viewer in Claude Desktop) |
| `screenshot`, `viewer_state`       | See what the user sees, plus recent errors               |
| `shader_guide`                     | The authoring guide                                      |

## Privacy and security

No data collection: see [PRIVACY.md](PRIVACY.md). Report vulnerabilities as described in
[SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE)
