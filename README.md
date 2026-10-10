# Shader Playground

A 3D playground for [WESL](https://wesl-lang.dev) shaders, built on Bevy 0.20, driven by
[MCP](https://modelcontextprotocol.io). Ask Claude Code to write shaders, get compile errors back
automatically, then fly around the results, in the browser or as a native app.

> *"Generate 5 variants of portal shaders"* → five projects in a `Portals` group, compiled
> and listed in the viewer. Open one, or show the whole group side by side.

## Features

- **Fly-through viewer**: free camera, a project dropdown, and a group view that lays out
  every project in the group with floating labels.
- **Runs in the browser or natively**: WebGPU/WebGL2 build on GitHub Pages with projects in
  browser storage, or a native app working on a folder.
- **Claude Code plugin and built-in MCP server**: agents create, edit, compile and delete projects,
  whether or not those projects are open in the viewer.
- **Real compile feedback**: projects compile in an off-screen sandbox, which reports WESL and
  naga errors, missing imports and asset failures, with an optional preview image.
- **Hot reload**: edits made by hand or through MCP show up immediately.
- **Batteries included**: material bindings, noise helpers, procedural textures, primitive,
  glTF and custom meshes.

## Quick start: browser

1. Install the Claude Code plugin (needs Node.js 18+):

   ```
   /plugin install wesl-playground --marketplace vovac12/wesl-playground
   ```

   On older Claude Code versions: `/plugin marketplace add vovac12/wesl-playground`, then
   `/plugin install wesl-playground@wesl-playground`.

2. Open the viewer: **https://vovac12.github.io/wesl-playground/**
3. Run `claude` and ask for some shaders.

The plugin runs a small local bridge (`plugin/server/bridge.mjs`) that Claude talks to over
stdio. The page connects to it at `ws://127.0.0.1:7880`. Projects are stored in the browser
(IndexedDB). Use `?bridge=ws://host:port` on the page URL and `WESL_PLAYGROUND_PORT` for the
bridge to change the port. Chrome may ask to allow the page to access devices on your local
network the first time.

## Quick start: native

Requires Rust 1.97+.

```sh
cargo run --release
```

Then run `claude` in this repo (it ships a `.mcp.json`) or register the server once:

```sh
claude mcp add --transport http shadertoy http://127.0.0.1:7878/mcp
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
    swirl/
      project.json
      portal.wesl
  my_project/              ungrouped project
```

A project is a folder containing `project.json`. Folder names are `snake_case`, so files
can import each other with `import super::common::foo;`.

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
mesh shapes and a vertex shader template. The MCP server also sends this guide to agents.

## MCP tools

| Tool                               | Description                                              |
| ---------------------------------- | -------------------------------------------------------- |
| `create_project`, `update_project` | Write manifest and files, compile, return errors/preview |
| `compile_project`                  | Compile off-screen, return errors and a preview image    |
| `list_projects`, `get_project`     | Browse groups, manifests and sources                     |
| `delete_project`, `set_group`      | Manage projects and group metadata                       |
| `add_texture`, `import_file`       | Procedural/base64/local textures, `.glb` meshes          |
| `open_in_viewer`                   | Show a project or a whole group                          |
| `screenshot`, `viewer_state`       | See what the user sees, plus recent errors               |
| `shader_guide`                     | The authoring guide                                      |

## License

[MIT](LICENSE)
