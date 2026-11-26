# DGI Cache Studio

A Tauri desktop app for browsing and replacing supported Roblox cache assets, including database entries, wrapped cache files, and exported asset folders. The interface is TypeScript and the local file operations run in Rust.

## Use the app

1. DGI looks for the Roblox cache at startup on Windows. The Library checks for new files every three seconds, including when another app is in focus. Use **Scan cache** for a full rescan with a live file count. **Choose folder** remembers another folder for the next launch, and **Find Roblox cache** in Settings switches back to automatic discovery.
   **Clear cache** removes loose cached assets and cached HTTP responses in the selected Roblox cache database after confirmation, then rescans the folder. Active replacements, backups, Roblox bookkeeping, and the editing workspace are kept. Roblox may download assets again while it runs.
   The Library search shows matching assets in a dropdown. Use the arrow keys and **Enter** to open a result, or **Escape** to close the dropdown. Set **Light** or **Dark** under **Settings → Appearance**. DGI saves the choice for the next launch.
   Use **Filters** in the Library or **Changes → Add asset** to show or hide asset types. Both views share the same filter preferences. **Clear all** makes it easy to pick just one type, and **Show all** resets the filters.
2. Select an asset to preview an image, play audio, or scrub a KeyframeSequence on an R6 or R15 rig. Animation previews use Three.js and require WebGL 2. On Windows, DGI requests the Vulkan backend through ANGLE in WebView2. The player has speed and loop controls. Set a custom name to override the display name.
3. Choose a replacement file or enter a numeric Roblox asset ID. DGI loads Open Cloud keys from environment variables such as `ROBLOX_OPEN_CLOUD_API_KEY`, `ROBLOX_API_KEY`, and `RBX_API_KEY`, including Windows user and system variables. **Settings → Roblox downloads** shows the key source and lets you check again, enter a different key, or clear the active key for the session. Downloads require Asset Delivery access. See the [Roblox Asset Delivery reference](https://create.roblox.com/docs/cloud/reference/features/assets) and [API key guide](https://create.roblox.com/docs/cloud/auth/api-keys).
4. Open **Changes → Add asset** to select an existing asset or import a local file. Wrapped assets can be copied into **Editing workspace** for editing, including KTX2 textures converted to PNG. Replace the selected copy in its inspector. **Restore** and **Restore all** undo replacements in the selected folder or workspace. **Library** returns to the cache folder you were browsing.

DGI previews PNG, JPEG, GIF, WebP, BMP, supported KTX2 textures, Ogg Vorbis and Opus, MP3, WAV, FLAC, and XML or binary Roblox KeyframeSequences. Audio playback depends on the system webview's codecs. Roblox models, avatar and clothing objects, curve animations, and meshes are listed by type. Plain image bytes do not distinguish decals from other uses, so those files share the **Images & decals** filter. Supported wrapped images, audio, and KeyframeSequences can be replaced directly in the Library, including entries stored inside the cache database. Close Roblox Player before applying or restoring a cache replacement, then join again to load it. DGI preserves the HTTP wrapper, recomputes its body length and checksum, and updates the database size. KTX2 textures remain preview only in their source folder. Workspace copies are independent files. After editing a copy, use **Apply to Roblox cache** to locate its unique original by content and write the edit back. DGI refuses missing or ambiguous matches and retains the workspace file. Applying switches to the real cache entry in Changes, where Restore puts its first original back. An animation played in game may never be saved to the cache, and Roblox can evict cached files.

DGI uses names and source asset IDs stored in Roblox assets, along with embedded PNG and Ogg titles. Cache entries without name metadata use a type label and a short cache key. **Reset name** clears a custom name and returns to the discovered or default label. The full cache path is shown in the inspector.

The Library includes supported images, audio, animations, and meshes stored inline in the Roblox cache database as well as loose files. Supported images, audio, and animations expose replacement controls directly on the cache entry. Preview-only formats can still be copied into the editing workspace. Select a mesh to view its geometry, rotate it by dragging, or zoom with the mouse wheel. Mesh previews use a neutral material and show the mesh in its rest pose.

DGI writes backups and SHA-256 records to its local app data directory. It keeps the first original through subsequent replacements and app restarts. It checks the current file before writing and verifies the result afterward. If another process changes the file, DGI reports a conflict rather than overwriting those bytes. Replacement files are limited to 32 MiB.

## Develop

Rig mesh data stays on the machine that can access it through Studio. On Windows, DGI reads `studio-rig-meshes.json` from its app data directory (`%LOCALAPPDATA%\com.dgi.desktop`). The file uses `{ "version": 1, "R15": { "UpperTorso": { "vertices": [x, y, z, ...], "triangles": [a, b, c, ...] } } }` with zero-based triangle indices and vertices in the corresponding part's local Roblox stud coordinates. `R6` uses the same schema. Restart DGI after replacing the local file.

A local export can add `r6Rig` or `r15Rig` with `parent`, `size`, `c0`, and `c1` entries for each body part. Each CFrame is stored as three position values followed by nine row-major rotation values. Parts can include an RGB `color` array. A head can include interleaved `uv` coordinates (top-left origin) and a base64 `texturePng` atlas or `frontDecalPng` from Studio.

On Windows, install the supplied R6 ZIP with `npm run rig:install -- "C:\path\to\roblox-r6-rigged.zip"` after `npm ci`. The importer uses the FBX's six rigid bone regions, their pivots, and the included texture atlas. The preview matches the head's base color to the torso while keeping the atlas's expression. The importer updates R6 in the local rig pack, keeps the existing R15 data, and saves the previous pack as `studio-rig-meshes.json.before-fbx` the first time. Restart DGI to load it.

After installing R6, run `npm run rig:install:r15 -- "C:\path\to\roblox-r15-rigged-obj-file.zip"` to install the supplied R15 OBJ. The importer uses the 15 named mesh groups, aligns their rest positions with the R15 joint transforms, and copies the R6 face atlas onto the compatible R15 head UVs. It backs up the previous pack as `studio-rig-meshes.json.before-r15` the first time. Restart DGI to load it.

Install Node.js, stable Rust, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). On Windows, the custom title bar uses [Tauri Window Controls](https://github.com/insd47/tauri-plugin-window-controls) for native minimize, maximize, close, and Snap Layouts support. Other platforms use their standard window decorations.

~~~sh
npm ci
npm run tauri dev
~~~

~~~sh
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri build
~~~

## Windows builds

GitHub Actions builds the Windows x64 installer on pushes to `master` and pull requests targeting it. Open a successful **Build DGI desktop** run and download the `DGI-windows-x64` artifact. The artifact contains the setup executable.

See [AGENTS.md](AGENTS.md) for the Ghost Acrylic tokens, component rules, and verification workflow.
