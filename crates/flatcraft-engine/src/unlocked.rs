//! Long commands with the session lock held only to take a snapshot and to
//! store the result (R30), so a window keeps drawing while an agent
//! imports, traces, opens a project, plans or draws a preview.
//!
//! The pattern is the one of `project.save` and `document.check_sources`:
//!
//! 1. Under the lock: the agent activity, the preferences, and a snapshot
//!    of the document the command runs in (the active one, or the one the
//!    window started it for): a twin with the same id and revision (see
//!    `Session::scratch_of`), with copies of the profiles and preferences.
//! 2. Without the lock: the command itself runs on the snapshot, the same
//!    code [`Session::execute`] runs, so the result and the errors are the
//!    same for the same document.
//! 3. Under the lock again: if the document is still open at the same
//!    revision, with the same path and profiles, the result is stored
//!    (`DocState::adopt`): the same undo step, dirty flag, selection, last
//!    job and caches the command left. Otherwise an import, trace, reload
//!    or relink is applied to the document as it is now ([`rebase`]), and
//!    any other command runs once more on a new snapshot; when the
//!    document changed again it fails with [`changed_error`]. A closed
//!    document fails with [`closed`].
//!
//! `document.export` makes its bytes on the snapshot and writes the file
//! only after that check, without the lock. `project.open` reads no
//! document, so only its rebuild runs without the lock and the document
//! is placed at store time.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use flatcraft_core::document::Document;
use serde_json::Value;

use crate::commands::export::{ExportParams, PreparedExport, prepare_export};
use crate::commands::project::{OpenParams, install_open, load_open, load_open_bytes};
use crate::registry::Command;
use crate::session::{CommandError, Origin, Session, internal_error};

/// What a command that runs off the lock was doing, for the error when
/// the document changed on both tries ([`changed_error`]); `None` for a
/// command that runs under the lock.
///
/// Every command here retries once: nothing built on an older state is
/// stored or written, and the agent's request still holds on the new
/// state, so one more try usually wins.
/// - The imports, retrace, reload and relink change the document: their
///   result on an older state would undo the change made meanwhile.
/// - `job.preview` stores the plan as the last job, and an agent decides
///   on a send by its numbers.
/// - `job.simulate` and `job.compare_strategies` store the simulation as
///   the document's last one, and an agent decides on its grade.
/// - `tools.offset_series` places its row away from the document's
///   objects and is planned for the profiles; with `send` it is sent only
///   when both are as they were ([`series`]).
/// - `document.preview` and `document.check`: an agent decides its next
///   step from the picture or the issues, so they must show the document
///   as it is.
/// - `document.export` writes the file only when its bytes are of the
///   document as it is.
/// - `project.open` reads no document; only a change of the profiles (the
///   blade offset of the rebuild) makes it stale.
pub fn stale_policy(id: &str) -> Option<&'static str> {
    Some(match id {
        "object.import" => "the file was imported",
        "object.import_svg" => "the SVG was imported",
        "object.retrace" => "the image was traced",
        "object.reload" => "the files were read again",
        "object.relink" => "the file was read",
        "job.preview" => "the job was planned",
        "job.simulate" => "the job was simulated",
        "job.compare_strategies" => "the strategies were compared",
        "tools.offset_series" => "the offset series was planned",
        "document.preview" => "the preview was drawn",
        "document.check" => "the objects were checked",
        "document.export" => "the file was exported",
        "project.open" => "the project was opened",
        _ => return None,
    })
}

/// True if [`execute`] runs `id` off the session lock.
pub fn runs_unlocked(id: &str) -> bool {
    stale_policy(id).is_some()
}

/// The error of a command whose document changed on both tries.
pub fn changed_error(what: &str) -> CommandError {
    CommandError::Failed(format!(
        "The document changed while {what}, so the result was dropped. Try again."
    ))
}

/// How often a command runs at most when the document changes meanwhile.
const TRIES: usize = 2;

fn lock(session: &Mutex<Session>) -> std::sync::MutexGuard<'_, Session> {
    session.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What must stay the same between the snapshot and the store.
#[derive(Debug, Clone, PartialEq)]
struct Key {
    doc_id: u64,
    revision: u64,
    profiles_revision: u64,
    /// A save gives the document a path without changing its revision; a
    /// new link is relative to it.
    path: Option<PathBuf>,
}

impl Key {
    /// The key of the open document at `index`.
    fn of(s: &Session, index: usize) -> Self {
        let st = &s.docs[index];
        Self {
            doc_id: st.id(),
            revision: st.revision(),
            profiles_revision: s.profiles.revision(),
            path: st.path.clone(),
        }
    }
}

/// The first step of every command here, under the lock: what
/// [`Session::execute`] does before it runs a command.
fn begin(s: &mut Session, origin: Origin) {
    if origin == Origin::Agent {
        s.last_agent_activity = Some(std::time::Instant::now());
    }
    s.refresh_prefs(std::time::Instant::now());
}

/// Runs the hook of the test, if any, then `step`; a panic in either is
/// the plain internal error of `id`.
fn compute<T>(
    id: &str,
    hook: Option<crate::session::UnlockedHook>,
    step: impl FnOnce() -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(h) = &hook {
            (h.0)(id);
        }
        step()
    }))
    .unwrap_or_else(|_| Err(internal_error(id)))
}

/// Runs the registry command `id` like [`Session::execute`], with the
/// slow part off the session lock when [`runs_unlocked`] says so (else
/// simply under the lock).
pub fn execute(
    session: &Mutex<Session>,
    id: &str,
    params: Value,
    origin: Origin,
) -> Result<Value, CommandError> {
    execute_on(session, None, id, params, origin)
}

/// What [`execute_in`] says when the document is no longer the active one.
pub const NOT_ACTIVE: &str = "Another document became active, so nothing was changed.";

/// The error of a command that must run in the active document when
/// another one is active (a command the window runs under the lock).
pub fn not_active() -> CommandError {
    CommandError::Refused(NOT_ACTIVE.into())
}

/// The error of a command whose document was closed (or replaced, as a
/// pristine document is by an open) before its result could be stored.
pub fn closed(id: &str) -> CommandError {
    CommandError::Refused(
        match id {
            "object.import" | "object.import_svg" => {
                "The import was not added: its document was closed."
            }
            "object.retrace" => "The trace was not applied: its document was closed.",
            "object.reload" => "The files were not read again: their document was closed.",
            "object.relink" => "The file was not linked: its document was closed.",
            "document.export" => "The export was not written: its document was closed.",
            _ => "Nothing was changed: the document was closed.",
        }
        .into(),
    )
}

/// [`execute`] in the open document `doc_id`, active or not, and the
/// active document stays as it is (like [`Session::execute_in`]): for a
/// long command ([`runs_unlocked`]) the window started on the document
/// the person saw. A tab switch meanwhile never drops it. Fails with
/// [`closed`] when the document was closed before the result could be
/// stored. Any other command runs only while `doc_id` is active.
pub fn execute_in(
    session: &Mutex<Session>,
    doc_id: u64,
    id: &str,
    params: Value,
    origin: Origin,
) -> Result<Value, CommandError> {
    execute_on(session, Some(doc_id), id, params, origin)
}

/// The index of the document a command runs in: `doc` while it is open,
/// else the active one when no document was given.
fn target(s: &Session, doc: Option<u64>, id: &str) -> Result<usize, CommandError> {
    match doc {
        Some(d) => s.doc_index(d).map_err(|_| closed(id)),
        None => Ok(s.active),
    }
}

fn execute_on(
    session: &Mutex<Session>,
    doc: Option<u64>,
    id: &str,
    params: Value,
    origin: Origin,
) -> Result<Value, CommandError> {
    let Some(what) = stale_policy(id) else {
        let mut s = lock(session);
        return match doc {
            Some(d) if s.doc_index(d).is_err() => Err(closed(id)),
            // Only the long commands may run in a document that is not
            // active; any other (one that opens, closes or switches
            // documents) would break the active document.
            Some(d) if s.doc().id() != d => {
                debug_assert!(
                    false,
                    "only long commands run in a document that is not active, not {id}"
                );
                Err(internal_error(id))
            }
            Some(_) | None => s.execute(id, params, origin),
        };
    };
    match id {
        "project.open" => {
            let p: Result<OpenParams, _> = crate::commands::parse(params);
            open(session, origin, what, move || {
                let p = p.as_ref().map_err(Clone::clone)?;
                Ok(Load::File(p.path.clone()))
            })
        }
        "document.export" => split(
            session,
            doc,
            id,
            origin,
            what,
            |scratch| {
                let p: ExportParams = crate::commands::parse(params.clone())?;
                prepare_export(scratch, p, origin)
            },
            PreparedExport::write,
        ),
        "tools.offset_series" => series(session, doc, params, origin, what),
        _ if rebases(id) => rebase(session, doc, id, params, origin, what),
        _ => split(
            session,
            doc,
            id,
            origin,
            what,
            |scratch| scratch.execute(id, params.clone(), origin),
            Ok,
        ),
    }
}

/// True for the commands whose slow part depends only on their inputs
/// (files, bytes, settings, the blade offset) and that apply it to the
/// document as it is now: a change of another object meanwhile never
/// loses them (see [`rebase`]).
pub fn rebases(id: &str) -> bool {
    matches!(
        id,
        "object.import"
            | "object.import_svg"
            | "object.retrace"
            | "object.reload"
            | "object.relink"
    )
}

/// A command that runs on a snapshot off the lock with its slow steps
/// recorded (`crate::memo`). If the document is unchanged, that result is
/// stored as it is. Else the command runs again under the lock on the
/// current document with the slow steps replayed, which is cheap: an
/// import lands whatever happened to the other objects. Only when the
/// replay needs a slow step the record did not make (the target object's
/// bytes or settings, or the blade offset changed) is the result dropped
/// and the command tried once more.
fn rebase(
    session: &Mutex<Session>,
    doc: Option<u64>,
    id: &str,
    params: Value,
    origin: Origin,
    what: &str,
) -> Result<Value, CommandError> {
    // The document it runs in is pinned at the first snapshot, also for
    // an agent: a tab switch meanwhile never moves it to another one.
    let mut doc = doc;
    for _ in 0..TRIES {
        let (mut scratch, key, base, hook) = {
            let mut s = lock(session);
            begin(&mut s, origin);
            let index = target(&s, doc, id)?;
            doc = Some(s.docs[index].id());
            (
                s.scratch_of(index),
                Key::of(&s, index),
                s.docs[index].twin_base(),
                s.unlocked_hook.clone(),
            )
        };
        let mut memo = None;
        let result = compute(id, hook, || {
            let (r, m) = crate::memo::record(|| scratch.execute(id, params.clone(), origin));
            memo = Some(m);
            r
        });
        let mut s = lock(session);
        let twin = scratch
            .docs
            .pop()
            .expect("a scratch session has one document");
        let Ok(index) = s.doc_index(key.doc_id) else {
            s.prune_pending();
            return Err(closed(id));
        };
        if Key::of(&s, index) == key {
            s.docs[index].adopt(&base, twin);
            s.prune_pending();
            return result;
        }
        let Some(memo) = memo else {
            // The snapshot run panicked: its error stands.
            s.prune_pending();
            return result;
        };
        // Apply on the document as it is now, with the cut pieces the
        // snapshot run built (its new object's among them) as a seed.
        let mut current = s.scratch_of(index);
        if let Some(g) = twin.cached_cut_geometry() {
            current.docs[0].seed_cut_geometry(&g);
        }
        let current_base = s.docs[index].twin_base();
        let mut missed = false;
        let applied = compute(id, None, || {
            let (r, m) = crate::memo::replay(memo, || current.execute(id, params.clone(), origin));
            missed = m;
            r
        });
        if !missed {
            let twin = current
                .docs
                .pop()
                .expect("a scratch session has one document");
            s.docs[index].adopt(&current_base, twin);
            s.prune_pending();
            return applied;
        }
        s.prune_pending();
    }
    Err(changed_error(what))
}

/// [`crate::commands::project::open_bytes`] with the rebuild off the
/// session lock: a sample project the window opens. `what` names it in
/// errors.
pub fn open_bytes(
    session: &Mutex<Session>,
    bytes: std::sync::Arc<[u8]>,
    what: &str,
    origin: Origin,
) -> Result<Value, CommandError> {
    let what_text = what.to_string();
    open(session, origin, "the project was opened", move || {
        Ok(Load::Bytes(bytes.clone(), what_text.clone()))
    })
}

/// The three steps of the module doc: `run` on a snapshot without the
/// lock, the check and the store under it, then `finish` (a file write)
/// without it again, only for a result that was stored.
fn split<T>(
    session: &Mutex<Session>,
    doc: Option<u64>,
    id: &str,
    origin: Origin,
    what: &str,
    run: impl Fn(&mut Session) -> Result<T, CommandError>,
    finish: impl FnOnce(T) -> Result<Value, CommandError>,
) -> Result<Value, CommandError> {
    for _ in 0..TRIES {
        let (mut scratch, key, base, hook) = {
            let mut s = lock(session);
            begin(&mut s, origin);
            let index = target(&s, doc, id)?;
            (
                s.scratch_of(index),
                Key::of(&s, index),
                s.docs[index].twin_base(),
                s.unlocked_hook.clone(),
            )
        };
        let result = compute(id, hook, || run(&mut scratch));
        {
            let mut s = lock(session);
            let Ok(index) = s.doc_index(key.doc_id) else {
                s.prune_pending();
                if doc.is_some() {
                    return Err(closed(id));
                }
                continue;
            };
            // An agent's command reads the active document: another one
            // active now is a change too.
            let fresh = Key::of(&s, index) == key && (doc.is_some() || index == s.active);
            if fresh {
                let twin = scratch
                    .docs
                    .pop()
                    .expect("a scratch session has one document");
                s.docs[index].adopt(&base, twin);
            }
            s.prune_pending();
            if !fresh {
                continue;
            }
        }
        let done = result?;
        return compute(id, None, || finish(done));
    }
    Err(changed_error(what))
}

/// `tools.offset_series` off the lock (`crate::calibrate`): the snapshot
/// and the checks under the lock, the planning and the simulation of the
/// tiles without it, then the reply and the send under it again, on the
/// real session, so a send that waits for the person is the session's
/// pending send. It runs on a document as it was snapshot; when the
/// document or the profiles changed meanwhile, it runs once more.
fn series(
    session: &Mutex<Session>,
    doc: Option<u64>,
    params: Value,
    origin: Origin,
    what: &str,
) -> Result<Value, CommandError> {
    let id = "tools.offset_series";
    for _ in 0..TRIES {
        let (input, send, key, hook) = {
            let mut s = lock(session);
            begin(&mut s, origin);
            let p: crate::commands::tools::OffsetSeriesParams =
                crate::commands::parse(params.clone())?;
            let index = target(&s, doc, id)?;
            let active = std::mem::replace(&mut s.active, index);
            let input = crate::calibrate::prepare(&s, &p.offsets_mm, p.send, origin);
            s.active = active;
            (input?, p.send, Key::of(&s, index), s.unlocked_hook.clone())
        };
        let run = compute(id, hook, || crate::calibrate::compute(input));
        let mut s = lock(session);
        let Ok(index) = s.doc_index(key.doc_id) else {
            s.prune_pending();
            if doc.is_some() {
                return Err(closed(id));
            }
            continue;
        };
        // An agent's command reads the active document: another one
        // active now is a change too.
        if Key::of(&s, index) != key || (doc.is_none() && index != s.active) {
            s.prune_pending();
            continue;
        }
        let finish = FinishSeries {
            run: Mutex::new(Some(run?)),
            send,
        };
        let active = std::mem::replace(&mut s.active, index);
        let result = s.run_prepared(&finish, origin);
        s.active = active.min(s.docs.len().saturating_sub(1));
        return result;
    }
    Err(changed_error(what))
}

/// A series planned and simulated off the lock, finished (the reply, the
/// send) as the command `tools.offset_series`, with its bookkeeping: a
/// panic in the send rolls the session back.
struct FinishSeries {
    run: Mutex<Option<crate::calibrate::SeriesRun>>,
    send: bool,
}

impl Command for FinishSeries {
    fn id(&self) -> &'static str {
        "tools.offset_series"
    }
    fn title(&self) -> &'static str {
        "Offset test series"
    }
    fn description(&self) -> &'static str {
        "Finishes an offset series planned off the session lock."
    }
    fn params_schema(&self) -> schemars::Schema {
        schemars::schema_for!(crate::commands::NoParams)
    }
    fn mutates_document(&self) -> bool {
        false
    }
    fn changes_session(&self) -> bool {
        true
    }
    fn run(&self, s: &mut Session, _: Value, origin: Origin) -> Result<Value, CommandError> {
        let run = self
            .run
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| internal_error("tools.offset_series"))?;
        crate::calibrate::finish(s, run, self.send, origin)
    }
}

/// What `project.open` reads: a file, or a project's bytes and the name
/// they go by in errors.
enum Load {
    File(PathBuf),
    Bytes(std::sync::Arc<[u8]>, String),
}

/// `project.open` off the lock: the file is read and its shapes rebuilt
/// without it, then the document is opened under it, with the bookkeeping
/// of every command that changes the session (a panic rolls back, linked
/// copies are checked). `what` parses the params under the lock, so a bad
/// param fails as it does under [`Session::execute`].
fn open(
    session: &Mutex<Session>,
    origin: Origin,
    changed: &str,
    what: impl Fn() -> Result<Load, CommandError>,
) -> Result<Value, CommandError> {
    let id = "project.open";
    for _ in 0..TRIES {
        let (load, profiles, hook) = {
            let mut s = lock(session);
            begin(&mut s, origin);
            (what()?, s.profiles.clone(), s.unlocked_hook.clone())
        };
        let loaded = compute(id, hook, || match &load {
            Load::File(path) => load_open(&profiles, path),
            Load::Bytes(bytes, name) => load_open_bytes(&profiles, bytes, name),
        });
        let mut s = lock(session);
        // A file's rebuild is stale when the profiles changed (the blade
        // offset of its traces). A sample built into the window opens
        // anyway: it is never stale, the person asked for that sample.
        if matches!(load, Load::File(_)) && s.profiles.revision() != profiles.revision() {
            s.prune_pending();
            continue;
        }
        let (document, warnings) = match loaded {
            Ok(l) => l,
            Err(e) => {
                s.prune_pending();
                return Err(e);
            }
        };
        let path = match load {
            Load::File(path) => Some(path),
            Load::Bytes(..) => None,
        };
        let install = Install {
            opened: Mutex::new(Some((path, document, warnings))),
        };
        return s.run_prepared(&install, origin);
    }
    Err(profiles_changed(changed))
}

/// The error of an open whose profiles changed on both tries.
pub fn profiles_changed(what: &str) -> CommandError {
    CommandError::Failed(format!(
        "The plotter or material profiles changed while {what}, so the result was dropped. Try again."
    ))
}

/// A project read and rebuilt off the lock: its file (none for bytes),
/// the document and the warnings.
type Opened = (Option<PathBuf>, Document, Vec<String>);

/// A project read and rebuilt off the lock, opened as the command
/// `project.open` (its id, so its errors read the same).
struct Install {
    opened: Mutex<Option<Opened>>,
}

impl Command for Install {
    fn id(&self) -> &'static str {
        "project.open"
    }
    fn title(&self) -> &'static str {
        "Open project"
    }
    fn description(&self) -> &'static str {
        "Opens a project read and rebuilt off the session lock."
    }
    fn params_schema(&self) -> schemars::Schema {
        schemars::schema_for!(crate::commands::NoParams)
    }
    fn mutates_document(&self) -> bool {
        false
    }
    fn changes_session(&self) -> bool {
        true
    }
    fn run(&self, s: &mut Session, _: Value, _: Origin) -> Result<Value, CommandError> {
        let (path, document, warnings) = self
            .opened
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| internal_error("project.open"))?;
        Ok(install_open(s, path, document, warnings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cutgeom::tests::OBJECT_CUTS;
    use crate::session::UnlockedHook;
    use std::sync::Arc;

    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="20" y="20" width="50" height="30"/></svg>"#;

    /// The apply step on a changed document cuts only what changed: the
    /// new object's pieces come from the snapshot run.
    #[test]
    fn the_apply_step_takes_the_new_objects_pieces_from_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let profiles = flatcraft_device::ProfileStore::load(&config).unwrap();
        let prefs = crate::prefs::Preferences {
            output_dir: Some(dir.path().join("output")),
            ..Default::default()
        };
        let s = Arc::new(Mutex::new(Session::new(profiles, prefs, false, config)));
        let a = s
            .lock()
            .unwrap()
            .execute(
                "object.import_svg",
                serde_json::json!({ "svg": SVG, "name": "a" }),
                Origin::User,
            )
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        s.lock().unwrap().cut_geometry().unwrap();
        let weak = Arc::downgrade(&s);
        s.lock().unwrap().unlocked_hook = Some(UnlockedHook(Arc::new(move |_: &str| {
            if let Some(s) = weak.upgrade() {
                s.lock()
                    .unwrap()
                    .execute(
                        "object.move",
                        serde_json::json!({ "ids": [a], "dx_mm": 1.0 }),
                        Origin::User,
                    )
                    .unwrap();
            }
        })));
        OBJECT_CUTS.with(|n| n.set(0));
        execute(
            &s,
            "object.import_svg",
            serde_json::json!({ "svg": SVG, "name": "b" }),
            Origin::Agent,
        )
        .unwrap();
        // The snapshot cuts b; the apply cuts the moved a, and b again
        // would make three.
        assert_eq!(OBJECT_CUTS.with(std::cell::Cell::get), 2);
    }
}
