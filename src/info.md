Welcome to
![Rick's Texture Ripper](logo_long_w.png)

Rip flat textures out of photos, un-warp their perspective, and pack them into a single atlas. Reopen this window any time from **Help > Info**.

## Getting started

- **Add an image** — File > Add Image (Ctrl+T) or the Texture View toolbar.
- **Add a rip** — File > Add Rip (Ctrl+R). A rip is a live selection that is un-warped into a flat texture and packed into the atlas.

## Texture View

- **Zoom** — mouse wheel / trackpad pinch (toward the cursor).
- **Pan** — hold the **Middle Mouse Button** and drag.
- **Move an image** — hold **Shift** and left-drag it.
- **Edit a rip** — plain left-drag adjusts the selected rip's handles.
- **Select** — left-click a rip (or an image).
- The transparency checkerboard stays anchored when toolbars change.

### Quad (perspective) & circle rips

- Drag a **corner** to warp the perspective freely; drag an **edge** to move that whole side.
- Drag **inside** the selection to move it.
- Switch a rip between **Quad** and **Circle**; **Remove Rip** / **Delete** removes it.
- Toggle subdivision **guide lines** with the top-right `#` icon or Edit > Guide Lines.
- **Edit > Cursor Interp** tunes the handle grab margin; **Edit > Preview Quality** trades live-preview sharpness for speed.

## Image Edit panel

- **Brightness / Contrast / Saturation** sliders, non-destructive.
- **Resize** the source image (its rips stay locked to the same features).
- Edits the **selected rip** if one is selected, otherwise the **active image**.

## Atlas panel

- **Padding** spaces packed rips apart.
- **Sort** — *Automatic* bin-packs everything tightly into the chosen aspect; *Manual* lets you **drag each rip** to position it.
  - Automatic scales rips **without** a custom output size toward a fair, even size; rips with a custom size are kept exact.
  - In Manual, enable **Snap** to snap a dragged rip to the grid and to nearby rip / canvas edges.
- **Aspect Ratio** (Automatic / Square / Custom) controls how **Width / Height** relate; rips are never stretched (the bounds just gain transparent padding).
- The preview **pans (middle-drag) and zooms (scroll)**; **Reset view** recenters.
- Drag the selected rip's **bottom-right grip** to set its output size — hold **Shift** to keep its aspect ratio.
- **Export** writes the atlas PNG at the chosen resolution.

## Projects

- **New / Open / Save / Save As** project files (`.rtrpf`). Projects are **self-contained** — the source images are embedded, so moving or deleting the originals won't break a saved project. Double-clicking a `.rtrpf` opens it.
- **File > Open Recent** lists recently used projects.
- **Undo / Redo** (Ctrl+Z / Ctrl+Y).
- **Window** menu toggles panels (Alt+1–4) and **Light Mode**.
- **Layout** menu saves, loads, and sets the startup panel arrangement.

## Hotkeys

### Navigation

- **Mouse wheel / pinch** — zoom toward the cursor
- **Middle-drag** — pan the view (Texture & Atlas)
- **Shift + left-drag** — move an image (Texture View)
- **Left-drag** — edit the selected rip's handles
- **Left-click** — select a rip / image
- **Shift** (while dragging the atlas resize grip) — lock aspect ratio

### Files & editing

- **Ctrl+T** Add Image · **Ctrl+R** Add Rip · **Ctrl+F** New Project
- **Ctrl+S** Save · **Ctrl+Shift+S** Save As · **Ctrl+X** Export Atlas
- **Ctrl+Z** Undo · **Ctrl+Y** / **Ctrl+Shift+Z** Redo
- **Delete** / **Backspace** — remove the selected rip, else the active image

### Panels

- **Alt+1** Texture View · **Alt+2** Atlas View · **Alt+3** Rips Gallery · **Alt+4** Image Edit

---

*Version 1.2 — written 2026-06-25.*
