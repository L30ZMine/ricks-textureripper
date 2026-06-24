![Rick's Texture Ripper](logo_long_w.png)

Rip flat textures out of photos and pack them into a single atlas.
This window can be reopened any time from **Help > Info**.

## Getting started

- **Add an image** — File > Add Image, or the button in the Texture View toolbar.
- **Add a rip** — File > Add Rip. A rip is a live selection that is un-warped into a flat texture and packed into the atlas.

## Texture View

- **Zoom** — mouse wheel / trackpad pinch (zooms toward the cursor).
- **Move an image** — hold **Shift** and left-drag the image.
- **Pan the view** — hold the **Middle Mouse Button**.
- **Edit a rip** — plain left-drag adjusts the selected rip's handles only.
- **Select** — left-click a rip to select it.

### Quad (perspective) rips

- Drag a **corner** to warp the perspective freely.
- Drag an **edge** to move that whole edge.
- Drag **inside** the selection to move the whole quad.
- Switch a rip between **Quad** and **Circle** in the Texture View toolbar.

### Guides

- Toggle the subdivision **guide lines** with the top-right `#` icon or **Edit > Guide Lines** (set how many lines there).

### Hard time grabbing handles?

- **Edit > Cursor Interp** tunes the handle grab margin.

## Image Edit panel

- **Brightness / Contrast / Saturation** sliders, non-destructive.
- **Resize** the source image (rips stay locked to the same features).
- Edits the **selected rip** if one is selected, otherwise the **active image**.

## Atlas panel

- **Padding** spaces packed rips apart.
- **Width / Height** set the export resolution (aspect-locked to the packed atlas).
- **Repack** re-runs packing; **Export PNG…** writes the atlas.
- Drag the selected rip's **bottom-right grip** in the preview to set its output size live.

## Project

- **New / Open / Save As** project files (`.rtrpf`). Source images are reloaded by path.
- **Export Atlas…** from the File menu.
- **Undo / Redo** from the Edit menu.
- **Layout** menu saves, loads, and sets the startup arrangement of the dockable panels.
