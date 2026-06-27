<p align="center">Rick's Texture Ripper - Built in Rust - Version 1.3.3 </p>
<p align="center">
  <img src="src/logo_long_g.png" alt="Rick's Texture Ripper" width="520">
</p>
Rick's Texture Rippper or RTR is a desktop tool for ripping flat textures out of photos and packing them into a texture atlas. Select a region in a photo, correct its perspective, and the result is added to a single atlas image you can export.

<p align="center">
<img src="screens/Screen%20(1).png" alt="Rick's Texture Ripper" width="320">
<img src="screens/Screen%20(2).png" alt="Rick's Texture Ripper" width="320">
</p>
<p align="center">
<img src="screens/Screen%20(3).png" alt="Rick's Texture Ripper" width="320">
<img src="screens/Screen%20(4).png" alt="Rick's Texture Ripper" width="320">
</p>

## Features

- **Perspective rips** — un-warp quads into flat textures
- **Circle rips** — round selections too
- **Live editing** — rips recompute as you drag
- **Automatic packing** — tight atlas, fewest holes
- **Manual placement** — drag rips, snap to grid
- **Aspect control** — Automatic, Square, or Custom
- **No stretching** — rips keep their proportions
- **Image adjustments** — non-destructive, per rip or image
- **Colour tools** — hue, temperature, gamma, tint
- **Filters** — sharpen and blur
- **Background removal** — colour key to transparent
- **Rip transforms** — rotate, flip, resize
- **Dockable panels** — arrange and save layouts
- **Projects** — self-contained `.rtrpf` files
- **Undo/redo** — full history per project
- **Drag and drop** — drop images to add
- **Autosave** — plus crash recovery
- **Fast** — multi-core background rendering
- **Recent files** — reopen past projects
- **Themes** — light and dark
- **Windows integration** — `.rtrpf` file association
- **Installer** — install, update, uninstall

## Usage

1. **Add Image** (Ctrl+T) to load a photo.
2. **Add Rip** (Ctrl+R) and drag the corners over the surface you want.
3. Open the **Atlas View** to arrange and size the rips.
4. **Export Atlas** (Ctrl+X) to write the PNG.

The in-app **Help > Info** window lists every control and shortcut.

## Navigation

- **Zoom** — mouse wheel
- **Pan** — middle mouse button
- **Move an image** — Shift + left-drag
- **Edit a rip** — left-drag its handles


## Build and run

Requires a stable Rust toolchain (`cargo`).

```
git clone https://github.com/L30ZMine/ricks-textureripper.git
cd ricks-textureripper
cargo run --release
```

The release build has no console window. A debug build (`cargo run`) keeps one for logging.

The application runs on Windows, Linux, and macOS. The executable icon and `.rtrpf` file association are Windows-only for now.


### Hotkeys

| Action | Shortcut |
| --- | --- |
| Add Image / Add Rip / New Project | Ctrl+T / Ctrl+R / Ctrl+F |
| Open Project | Ctrl+G |
| Save / Save As / Export Atlas | Ctrl+S / Ctrl+Shift+S / Ctrl+X |
| Undo / Redo | Ctrl+Z / Ctrl+Y |
| Remove selected rip or active image | Delete / Backspace |
| Toggle Texture / Atlas / Rips / Image Edit panels | Alt+1 / Alt+2 / Alt+3 / Alt+4 |
| Quit | Ctrl+Q |

## Tech stack

`eframe` + `egui` (GUI), `egui_dock` (panels), `image` (decode/encode), `rectangle-pack` (packing), `rfd` (file dialogs), `serde` (project and config files).

## License

No license has been specified yet.

## Author

[l30z](https://github.com/L30ZMine)
