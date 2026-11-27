//! The slow steps of a command that only depend on their inputs (reading
//! a file, hashing bytes, an SVG import, a trace), recorded while the
//! command runs on a snapshot off the session lock and replayed when it
//! runs again on the current document under the lock
//! (`crate::unlocked`). So the apply step under the lock is cheap, and it
//! sees the document as it is now: a change of another object never
//! loses an import.
//!
//! Every step is looked up by its whole input (a file by its path, bytes
//! by their content). A replay that needs a step the record did not make
//! (the target object's bytes or settings changed, or the blade offset)
//! never computes it under the lock: it gives a plain error instead and
//! notes the miss, so the caller drops that result and tries again. Without a memo (the normal case) each step simply
//! runs.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use flatcraft_core::document::TraceSettings;
use flatcraft_core::geometry::CutPath;
use flatcraft_import::svg::SvgImport;
use flatcraft_import::trace::TraceResult;
use flatcraft_import::{ImportError, fonts};

use crate::commands::sources::{ReadError, SourceFile};

/// Which fonts an SVG import sets text with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fonts {
    /// The fonts of this computer (`fonts::system_fonts`).
    System,
    /// The fonts the document names (`fonts::fonts_for`).
    ForDocument,
}

/// An SVG import whose paths are shared: the same `Arc` on every replay.
pub(crate) type Svg = (Arc<SvgImport>, Arc<Vec<CutPath>>);

/// A trace whose paths are shared.
pub(crate) type Traced = (Arc<TraceResult>, Arc<Vec<CutPath>>);

/// A recorded SVG import: its bytes, fonts and result.
type SvgStep = (Arc<[u8]>, Fonts, Result<Svg, ImportError>);

/// A recorded trace: its bytes, settings, blade offset (bits) and result.
type TraceStep = (Arc<[u8]>, TraceSettings, u64, Result<Traced, ImportError>);

/// The recorded steps of one command.
#[derive(Default)]
pub(crate) struct Memo {
    reads: Vec<(PathBuf, Result<SourceFile, ReadError>)>,
    hashes: Vec<(Arc<[u8]>, String)>,
    svgs: Vec<SvgStep>,
    traces: Vec<TraceStep>,
    /// A replay needed a step that was not recorded.
    missed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Record,
    Replay,
}

thread_local! {
    static ACTIVE: RefCell<Option<(Mode, Memo)>> = const { RefCell::new(None) };
}

/// Runs `f` with every slow step recorded; returns its result and the
/// memo.
pub(crate) fn record<T>(f: impl FnOnce() -> T) -> (T, Memo) {
    run(Mode::Record, Memo::default(), f)
}

/// Runs `f` with the slow steps taken from `memo`; returns its result
/// and whether a step was missing (the result must then be dropped).
pub(crate) fn replay<T>(memo: Memo, f: impl FnOnce() -> T) -> (T, bool) {
    let (out, memo) = run(Mode::Replay, memo, f);
    (out, memo.missed)
}

fn run<T>(mode: Mode, memo: Memo, f: impl FnOnce() -> T) -> (T, Memo) {
    struct Reset(Option<(Mode, Memo)>);
    impl Drop for Reset {
        fn drop(&mut self) {
            let previous = self.0.take();
            ACTIVE.with(|a| *a.borrow_mut() = previous);
        }
    }
    let previous = ACTIVE.with(|a| a.borrow_mut().replace((mode, memo)));
    let reset = Reset(previous);
    let out = f();
    let memo = ACTIVE
        .with(|a| a.borrow_mut().take())
        .map(|(_, m)| m)
        .unwrap_or_default();
    drop(reset);
    (out, memo)
}

/// Looks `find` up in the active memo; on a record, computes and stores
/// it; on a miss in a replay, computes it and notes the miss; without a
/// memo, computes it.
fn step<T: Clone>(
    find: impl Fn(&Memo) -> Option<T>,
    compute: impl FnOnce() -> T,
    missing: impl FnOnce() -> T,
    store: impl FnOnce(&mut Memo, T),
) -> T {
    let mode = ACTIVE.with(|a| a.borrow().as_ref().map(|(m, _)| *m));
    if let Some(found) = ACTIVE.with(|a| a.borrow().as_ref().and_then(|(_, m)| find(m))) {
        return found;
    }
    if mode == Some(Mode::Replay) {
        // The replay runs under the session lock: a missing step is never
        // computed there. The caller drops this result and tries again.
        ACTIVE.with(|a| {
            if let Some((_, m)) = a.borrow_mut().as_mut() {
                m.missed = true;
            }
        });
        return missing();
    }
    // The step runs without the memo borrowed.
    let value = compute();
    if mode == Some(Mode::Record) {
        ACTIVE.with(|a| {
            if let Some((_, m)) = a.borrow_mut().as_mut() {
                store(m, value.clone());
            }
        });
    }
    value
}

/// What a missing step gives in a replay: an error, whose result is
/// dropped anyway.
fn missing_import() -> ImportError {
    ImportError::Parse("not recorded".into())
}

fn same(a: &Arc<[u8]>, b: &[u8]) -> bool {
    std::ptr::eq(a.as_ptr(), b.as_ptr()) && a.len() == b.len() || **a == *b
}

/// [`crate::commands::sources::read_capped`].
pub(crate) fn read(path: &Path) -> Result<SourceFile, ReadError> {
    step(
        |m| {
            m.reads
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, r)| r.clone())
        },
        || crate::commands::sources::read_capped(path),
        || Err(ReadError::NotFound),
        |m, r| m.reads.push((path.to_path_buf(), r)),
    )
}

/// [`crate::commands::sources::sha256_hex`].
pub(crate) fn sha(bytes: &[u8]) -> String {
    step(
        |m| {
            m.hashes
                .iter()
                .find(|(b, _)| same(b, bytes))
                .map(|(_, h)| h.clone())
        },
        || crate::commands::sources::sha256_hex(bytes),
        String::new,
        |m, h| m.hashes.push((Arc::from(bytes), h)),
    )
}

/// An SVG import of `bytes` with `fonts`.
pub(crate) fn svg(bytes: &[u8], which: Fonts) -> Result<Svg, ImportError> {
    step(
        |m| {
            m.svgs
                .iter()
                .find(|(b, f, _)| *f == which && same(b, bytes))
                .map(|(_, _, r)| r.clone())
        },
        || {
            let fonts = match which {
                Fonts::System => fonts::system_fonts(),
                Fonts::ForDocument => fonts::fonts_for(bytes),
            };
            flatcraft_import::svg::import_svg(bytes, fonts).map(|i| {
                let paths = Arc::new(i.paths.clone());
                (Arc::new(i), paths)
            })
        },
        || Err(missing_import()),
        |m, r| m.svgs.push((Arc::from(bytes), which, r)),
    )
}

/// A trace of `bytes` with `settings` for the blade offset `blade_mm`.
pub(crate) fn trace(
    bytes: &[u8],
    settings: &TraceSettings,
    blade_mm: f64,
) -> Result<Traced, ImportError> {
    let blade = blade_mm.to_bits();
    step(
        |m| {
            m.traces
                .iter()
                .find(|(b, s, d, _)| *d == blade && s == settings && same(b, bytes))
                .map(|(_, _, _, r)| r.clone())
        },
        || {
            flatcraft_import::trace::trace(bytes, settings, blade_mm).map(|t| {
                let paths = Arc::new(t.paths.clone());
                (Arc::new(t), paths)
            })
        },
        || Err(missing_import()),
        |m, r| {
            m.traces
                .push((Arc::from(bytes), settings.clone(), blade, r))
        },
    )
}
