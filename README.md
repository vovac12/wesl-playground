# Shader Playground

A 3D playground for [WESL](https://wesl-lang.dev) shaders, built on Bevy 0.20, with a built-in
[MCP](https://modelcontextprotocol.io) server. Ask an AI agent like Claude Code to write shaders,
get compile errors back automatically, then fly around the results.

> *"Generate 5 variants of portal shaders"* → five projects in a `Portals` group, compiled
> and listed in the viewer. Open one, or show the whole group side by side.

## Features

- **Fly-through viewer**: free camera, a project dropdown, and a group view that lays out
  every project in the group with floating labels.
- **MCP server built into the app**: agents create, edit, compile and delete projects,
  whether or not those projects are open in the viewer.
- **Real compile feedback**: projects compile in an off-screen sandbox, which reports WESL and
  naga errors, missing imports and asset failures, with an optional preview image.
- **Hot reload**: edits made by hand or through MCP show up immediately.
- **Batteries included**: material bindings, noise helpers, procedural textures, primitive,
  glTF and custom meshes.

## Quick start

Requires Rust 1.97+.

```sh
cargo run --release
```

Then connect Claude Code, either by running `claude` in this repo (it ships a `.mcp.json`) or
by registering the server once:

```sh
claude mcp add --transport http shadertoy http://127.0.0.1:7878/mcp
```

| Env var       | Default          | Purpose                     |
| ------------- | ---------------- | --------------------------- |
| `BASE_FOLDER` | `./projects`     | Where shader projects live  |
| `MCP_ADDR`    | `127.0.0.1:7878` | MCP server address (`/mcp`) |

On first launch the app creates an `examples` group in an empty `BASE_FOLDER`.

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

See [`src/guide.md`](src/guide.md) for the full manifest format, bindings, mesh shapes and a
vertex shader template. The MCP server also sends this guide to agents.

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
