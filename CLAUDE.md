# Rick's Texture Ripper — project state

A Rust desktop app (texture ripper / atlas tool) inspired by Rick's Texture Ripper.

## Stack (non-negotiable)
- `eframe` + `egui` 0.31 — GUI
- `egui_dock` 0.16 (with `serde` feature) — Blender-style dockable panels
- `image` 0.25 — load/edit
- `rectangle-pack` 0.4 — atlas bin-packing
- `rfd` 0.15 — native file dialogs
- `serde` / `serde_json` / `dirs` — layout & config persistence

## Build / run (Windows, this machine)
- `cargo` is NOT on PATH automatically. Prepend it: `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"`.
- Toolchain: stable MSVC (rustc 1.96.0), installed via `winget install Rustlang.Rustup`.
- Build: `cargo build`. Run: `cargo run`.
- To test launch without blocking: `Start-Process` cargo run, sleep ~9s, then `Get-Process -Name ricks-textureripper | Stop-Process -Force`.
- The user prefers to run builds themselves via `!cargo run` in the prompt.

## Module layout (`src/`)
- `main.rs` — eframe bootstrap, module decls.
- `app.rs` — `App`: combined top bar (File/Edit/Layout/Help menus **+ inline project tabs**, same row, vertically centered via `left_to_right(Align::Center)`; tabs drawn inline with no group frame so they share the menu centerline, and show a trailing `*` when `project.modified`), About + Save-Layout windows. Per-frame update: detects `busy` (pointer down) to drive **preview-quality** rip extraction, reruns full-res on settle, then `repack_if_needed`, then commits undo history when idle. File menu: Add Image/Rip, New/Open/Save As, **Export Atlas…** (calls `atlas::export`), Exit. Edit menu: Undo/Redo, **Guide Lines** submenu, **Cursor Interp** submenu (slider → `project.cursor_margin`). New projects are named `unnamed`; **Save As** renames the tab to the file stem and clears `modified`. Undo/Redo set `modified`.
- `project.rs` — `Project` (owns `dock_state`, `history`, `needs_full`, **`modified`** unsaved-flag, **`cursor_margin`** handle hit-test margin px), `LoadedImage` (keeps `original` pixels + `adjust` + `dirty` + `source_path` + **`mips`** downscale chain; `preview_source(target_scale)` picks the smallest mip ≥ scale), `Adjustments`, `Rip`/`RipOutput` (rip has `adjust` + `resize`), `Guides`, `Atlas`/`AtlasSettings` (**`padding` + aspect-locked `export_w`/`export_h`**; `0` = follow natural)/`AtlasResult`/`AtlasPlacement`, `ViewState` (incl. `dragging_image` + **`panning`**). `reset_history`/`commit_history_if_changed` (sets `modified` on real change) bridge to `snapshot`. Serde derives on `Adjustments`/`AtlasSettings`/`Guides`. (`modified`/`cursor_margin` are runtime-only, not serialized.)
- `snapshot.rs` — `ProjectSnapshot` (+ `ImageState`/`RipState`/`SerShape`): serializable document state shared by undo/redo and `.rtrpf`. `capture(project)` and `restore(ctx, project, snap)` (reuses loaded images by `source_path`, reloads missing). `same_document` ignores selection/active so clicking around isn't undoable.
- `history.rs` — `History`: baseline + undo/redo stacks of `ProjectSnapshot`, `commit` (push when document changed; **returns `bool`** = whether it committed), `undo`/`redo`, capped at 100.
- `proj_io.rs` — `.rtrpf` ("Rick's Texture Ripper Project File") save/open: JSON `{version, name, snapshot, dock_state}`. `EXTENSION = "rtrpf"`. `open` clears `modified`.
- `ui/docking.rs` — `PanelTab` (serde): Atlas / Texture / ImageEdit / **Rips** (Rips is its own dockable panel now). `DockViewer` (egui_dock `TabViewer`); rip thumbnail gallery (clicking the thumbnail selects the rip).
- `texture_view.rs` — pannable/zoomable canvas, `load_loaded_image` (sets `source_path`, builds `mips`), **Shift+left-drag** moves the image under the cursor (empty space pans); **plain left-drag only adjusts rip handles** (so the cursor never lies); live rip editing, handle cursor feedback (Grab while Shift held), guide-toggle icon (top-right), guide overlay, `upload_texture`. Passes `project.cursor_margin` into hit-testing.
- `rip_tool.rs` — `RipShape` (Quad([Pos2;4]) | Circle), `RipEditor`, `DragHandle`, `Xform`, hit-testing **`hit_handle(rip, x, ptr, margin)`** (corner grab radius = `margin`; **edges grabbable along the segment except within `margin` of a vertex** via `closest_point_on_segment`; whole-selection move only ≥`margin` inside), drag apply, draw, `draw_guides`, `handle_cursor` (corners→Crosshair, edges→Resize H/V, move→Move), `recompute_dirty(ctx, project, preview)`. Preview now warps a **downscaled mip** (`preview_source`, corners pre-scaled) at `PREVIEW_SCALE` then resizes back up to the target size, so the **atlas footprint stays stable**; resize override applies in preview too; sets `needs_full`.
- `image_edit.rs` — Phase 5: `apply_adjustments`, `resize_to`, **`build_mips`** (≤4 half-steps down to >64px), `recompute_dirty_images` (rebuilds `mips`), Image Edit panel UI (edits selected rip, else active image; `Width`/`Height` fields with ` px` suffix).
- `warp.rs` — perspective un-warp: homography (8x8 solve) + bilinear sampling; `unwarp_quad(src, corners, scale)` (scale<1 = cheaper preview res); **`natural_size(corners)`** = full-res output dims (used to keep preview footprint stable).
- `atlas.rs` — bin-pack (large fixed `MAX_BIN`, no max-dim setting) + composite + settings UI (`padding` + aspect-locked export `Width`/`Height`, all ` px`) + interactive preview (selected rip gets a **bottom-right resize grip** → sets `rip.resize` live) + **`export`** (pub; scales atlas to export resolution) + `export_size`.
- `layouts.rs` — named layout save/load/delete (JSON), built-in immutable `default` (now Atlas TL / ImageEdit BL / Texture TR / Rips BR), `Config` (default_layout). Under `Documents/ricks-textureripper/`.

## Done (Phases 1–5 + extras)
- Phase 1: skeleton, menu bar, dockable panels, project tabs.
- Phase 2: Texture View — add image(s), multiple images, pan+zoom, per-image drag.
- Phase 3 (extended): live rips — **perspective-warp quad** (free corners + edge handles) and circle; no Extract button, recomputed live.
- Phase 4: atlas bin-packing, live repack, padding, preview, Export PNG. (Max-dim & power-of-two settings **removed**; bin is a large fixed `MAX_BIN`.)
- Phase 5: Image Edit panel — brightness/contrast/saturation + resize, non-destructive (images keep `original`), live preview; edits selected rip else active image. Rips support an output-size override; resizing a source image rescales its rips.
- **Project save/open** (`.rtrpf`) via `proj_io` + `snapshot`; images reloaded by `source_path`. Tabs named `unnamed` + `*` when modified; Save As names the tab after the file and clears the `*`.
- **Undo/Redo** via `history` + `snapshot` (committed when the user settles; selection/pan not tracked).
- **Atlas export resolution:** aspect-locked `Width`/`Height` (edit one, the other follows the packed aspect); export PNG scales to it. **File > Export Atlas…** and the panel's Export PNG both go through `atlas::export`.
- **Live per-rip output size on the atlas:** drag the selected rip's bottom-right grip in the Atlas preview to set its `resize` live.
- **Performance / mipmaps:** display scaling is GPU (egui linear-filtered textures); the CPU perspective warp falls back to a reduced-resolution preview while dragging. Source images now carry a `mips` chain; the preview warp samples the smallest mip ≥ `PREVIEW_SCALE` and scales the result back up to the natural/override size — cheaper, cache-friendly/anti-aliased, and the **atlas footprint stays stable** during drags (`needs_full` reruns full-res on settle). Image brightness/contrast/saturation adjustments still run full-res each frame.
- Per-project dock layouts + Layout menu (Save/Load/**Set Initializing Layout**/Delete) + persistence. Built-in `default` is read-only; setting a custom initializing layout creates an editable project from it.
- UX: combined menu-bar + project-tab row (both vertically centered on the same line); **Shift+left-drag moves images / pans, plain left-drag is rip-only** (cursor never lies); **tunable handle hit-test margin** (Edit > Cursor Interp, default 5px) controlling corner grab radius, edge dead-zone around vertices, and move-region inset; guide lines for the selected quad (Edit > Guide Lines or top-right `#` icon); handle-aware cursors (crosshair on vertices, resize along edges, Grab while Shift held); clicking empty canvas keeps the active image selected; **Rips is its own dockable panel** (clicking a thumbnail selects the rip).

## NOT done / next up
- Project file is **Save As** only (no current-path tracking / "Save" overwrite, no recent-files). `.rtrpf` does not embed pixels — moving/deleting a source image breaks reload.
- `.rtrpf` `AtlasSettings` changed (export_w/export_h replaced max_w/max_h/power_of_two) — older project files won't deserialize their atlas settings.
- Undo history is per-project and not persisted; selection/pan/zoom are not undoable.
- Image-adjustment preview is not downscaled (only the perspective warp uses mips) — large-image slider drags do a full pass per frame.
- Atlas resize grip is a single bottom-right handle; export resolution doesn't re-derive after a repack changes the packed aspect (can distort until re-edited).

## Conventions
- Builds must compile AND run before moving to the next phase; user works incrementally.
- Keep modules separate (ui/docking, project, texture view, rip tool, atlas, image edit, warp, snapshot, history, proj_io).
- Branding string: always "ricks-textureripper" or "Rick's Texture Ripper".
- User-path saves go under `Documents/ricks-textureripper/`. Project files are `.rtrpf`.
- Repo is git-initialized (`master`), `target/` gitignored. Do not commit unless asked.
