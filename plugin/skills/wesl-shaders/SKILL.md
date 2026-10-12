---
name: wesl-shaders
description: Use when writing, editing or debugging WESL/WGSL shaders in the WESL Playground, including creating shader projects, fixing compile errors, making variants of an effect, or showing a shader in the 3D viewer.
---

# Writing shaders in the WESL Playground

The WESL Playground MCP tools store each shader project as a folder (a `project.json` plus
`.wesl` files) and compile it in a viewer that runs in the user's browser or inline in the chat.

## Workflow

1. **Read the guide first.** Call `shader_guide` once per conversation before writing any
   shader. It has the manifest format, the WESL import rules, the bindings you must import
   (never declare your own `@group/@binding`) and fragment/vertex templates. Follow it rather
   than general WGSL or Bevy knowledge, which is often out of date here.
2. **Create or update the project.** New work goes through `create_project`; changes to an
   existing project go through `update_project` (call `get_project` first if you did not write
   it). Both compile the project and return errors.
3. **Compile until clean.** Read every error, fix the cause and call `update_project` again
   until the result reports `ok: true`. WESL parse errors point at your file and line; naga
   errors (`wgsl:L:C`) refer to the composed shader, so find them by the quoted code. Pass
   `preview: true` when you want an image of the result without changing what the user sees.
4. **Variants belong in a group.** Use ids like `portals/swirl`, `portals/rings`, and give the
   group a name with `set_group`. Put code the variants share (SDFs, palettes, noise) in the
   group's `shared/` folder with `update_shared_files` instead of copying it into each project.
5. **Check the result.** When the user wants to see it, call `open_in_viewer` with the project
   or the whole group, then `screenshot` to check that it looks as intended. Use
   `viewer_state` for runtime errors and warnings.

## When no viewer is connected

If a tool reports that no viewer is running, tell the user how to start one:

- **Claude Code:** open https://vovac12.github.io/wesl-playground/ in a browser and keep the tab
  visible (browsers pause hidden tabs), then retry.
- **Claude Desktop:** call `open_in_viewer`; it starts the viewer inline in the chat.

Every tool except `shader_guide` needs a viewer, because the viewer does the compiling.
