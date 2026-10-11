# Privacy policy

WESL Playground (the Claude Desktop extension, the Claude Code plugin, the web viewer and the
native app) does not collect, transmit or sell any personal data.

- **Projects stay on your machine.** Shaders, manifests, textures and meshes are written to
  your projects folder (default `~/.wesl-playground/projects`), or to your browser's IndexedDB
  when the web viewer runs without the bridge. Nothing is uploaded anywhere.
- **No analytics, telemetry or accounts.** The software contains no tracking code and needs no
  sign-in.
- **No conversation data.** The bridge only receives the arguments of its own tool calls. It
  does not read your chat history, memory or uploaded files.
- **Local files** are read only when a tool call names one (`source_path` in `add_texture` or
  `import_file`), and only images and meshes (`.png`, `.jpg`, `.jpeg`, `.ktx2`, `.hdr`,
  `.glb`, `.gltf`, `.bin`) can be copied that way.
- **Network.** The viewer's code is loaded from GitHub Pages
  (`https://vovac12.github.io/wesl-playground/`). GitHub serves those files and may log
  requests under the [GitHub Privacy Statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement).
  The bridge listens only on `127.0.0.1` and accepts browser connections only from the
  viewer's origin.

Retention: there is nothing to retain on our side. Delete your projects folder to remove
everything the software stored.

Questions: open an issue at https://github.com/vovac12/wesl-playground/issues.
