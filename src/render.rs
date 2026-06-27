//! Background workers for the two heaviest CPU jobs, so the UI thread stays
//! responsive:
//!
//! - [`RipRenderer`] computes full-resolution rip outputs (perspective un-warp +
//!   colour/filters/resize/orient) off-thread. The UI keeps showing the cheap
//!   live preview, then swaps in the crisp result when it's ready. Grabbing a
//!   handle again cancels the in-flight job and goes back to previewing.
//! - [`ImageLoader`] decodes added image files off-thread, reporting progress so
//!   the app can show a small percentage by the cursor instead of freezing.
//!
//! Both use plain `std::thread` + channels (no extra dependency).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use image::RgbaImage;

use crate::project::{Adjustments, Orientation};
use crate::rip_tool::RipShape;

// ---------------------------------------------------------------------------
// Rip full-resolution rendering
// ---------------------------------------------------------------------------

/// One rip to render at full resolution. `src` is a shared copy of the source
/// image's working pixels (cloned once per image when the job is built).
pub struct RipRenderInput {
    pub rip: usize,
    pub src: Arc<RgbaImage>,
    pub shape: RipShape,
    /// Curved-quad mode: `false` = un-warp to a rectangle, `true` = mask/cut-out.
    pub bezier_shape: bool,
    pub adjust: Adjustments,
    pub orient: Orientation,
    pub resize: Option<[u32; 2]>,
}

/// A finished (or skipped) rip render, tagged with the job generation it came
/// from so stale results (from a cancelled job) can be discarded.
pub struct RipRenderMsg {
    pub rip: usize,
    pub generation: u64,
    /// `Some((size, pixels))` on success, `None` if the rip was degenerate.
    pub result: Option<([usize; 2], RgbaImage)>,
}

/// Runs full-resolution rip renders on a background thread.
pub struct RipRenderer {
    rx: Option<Receiver<RipRenderMsg>>,
    cancel: Arc<AtomicBool>,
    generation: u64,
    /// Number of results still expected for the current job.
    pending: usize,
}

impl Default for RipRenderer {
    fn default() -> Self {
        Self {
            rx: None,
            cancel: Arc::new(AtomicBool::new(false)),
            generation: 0,
            pending: 0,
        }
    }
}

impl RipRenderer {
    /// Starts a new background job, cancelling any in-flight one. Each input is
    /// rendered in turn; results are tagged with a fresh generation.
    pub fn start(&mut self, inputs: Vec<RipRenderInput>) {
        // Signal the previous job to stop, then detach from its channel.
        self.cancel.store(true, Ordering::Relaxed);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        self.pending = inputs.len();
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            for input in inputs {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let result = crate::rip_tool::render_full(
                    &input.src,
                    &input.shape,
                    input.bezier_shape,
                    &input.adjust,
                    &input.orient,
                    input.resize,
                );
                let msg = RipRenderMsg {
                    rip: input.rip,
                    generation,
                    result,
                };
                if tx.send(msg).is_err() {
                    break; // receiver gone (cancelled / replaced)
                }
            }
        });
    }

    /// Cancels the in-flight job (if any) and drops its results.
    pub fn cancel(&mut self) {
        if self.rx.is_some() {
            self.cancel.store(true, Ordering::Relaxed);
            self.rx = None;
            self.pending = 0;
        }
    }

    /// True while a job is still producing results.
    pub fn is_active(&self) -> bool {
        self.rx.is_some() && self.pending > 0
    }

    /// Drains the results available so far (only those from the current job).
    pub fn poll(&mut self) -> Vec<RipRenderMsg> {
        let mut out = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(msg) = rx.try_recv() {
                if msg.generation == self.generation {
                    self.pending = self.pending.saturating_sub(1);
                    out.push(msg);
                }
            }
        }
        if self.pending == 0 {
            self.rx = None;
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Background image loading
// ---------------------------------------------------------------------------

/// A decoded image, ready for the UI thread to upload as a texture. `original`
/// and `pixels` start equal; both are produced off-thread so the UI doesn't have
/// to clone a large buffer.
pub struct LoadedImageData {
    pub name: String,
    pub source_path: PathBuf,
    pub original: RgbaImage,
    pub pixels: RgbaImage,
    pub mips: Vec<RgbaImage>,
}

/// Result of decoding one added image file.
pub enum ImageLoadMsg {
    Loaded(Box<LoadedImageData>),
    Failed(String),
}

/// Decodes added image files on a background thread, tracking progress.
#[derive(Default)]
pub struct ImageLoader {
    rx: Option<Receiver<ImageLoadMsg>>,
    /// Project index the images are being added to.
    pub target: usize,
    done: usize,
    total: usize,
}

impl ImageLoader {
    /// Begins decoding `paths` into project `target` (replaces any prior batch).
    pub fn start(&mut self, target: usize, paths: Vec<PathBuf>) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.target = target;
        self.done = 0;
        self.total = paths.len();
        std::thread::spawn(move || {
            for path in paths {
                if tx.send(decode_image(&path)).is_err() {
                    break;
                }
            }
        });
    }

    /// Drains decoded results available so far.
    pub fn poll(&mut self) -> Vec<ImageLoadMsg> {
        let mut out = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(m) = rx.try_recv() {
                self.done += 1;
                out.push(m);
            }
        }
        if self.done >= self.total {
            self.rx = None;
        }
        out
    }

    /// True while images are still being decoded.
    pub fn active(&self) -> bool {
        self.rx.is_some() && self.done < self.total
    }

    /// Fraction decoded so far, `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.done as f32 / self.total as f32
        }
    }
}

/// Decodes one image file (off-thread): RGBA + mip chain + a working-pixel copy.
fn decode_image(path: &PathBuf) -> ImageLoadMsg {
    match image::open(path) {
        Ok(dynimg) => {
            let rgba = dynimg.to_rgba8();
            let mips = crate::image_edit::build_mips(&rgba);
            let pixels = rgba.clone();
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "image".to_string());
            ImageLoadMsg::Loaded(Box::new(LoadedImageData {
                name,
                source_path: path.clone(),
                original: rgba,
                pixels,
                mips,
            }))
        }
        Err(e) => ImageLoadMsg::Failed(format!("Failed to load {}: {e}", path.display())),
    }
}
