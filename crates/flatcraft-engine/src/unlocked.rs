//! Long commands with the session lock held only to take a snapshot and to
//! store the result (R30), so a window keeps drawing while an agent
//! imports, traces, opens a project, plans or draws a preview.
//!
//! The pattern is the one of `project.save` and `document.check_sources`:
//!
//! 1. Under the lock: the agent activity, the preferences, and a snapshot
//!    of the active document (a twin with the same id and revision, see
//!    `Session::scratch`) with copies of the profiles and preferences.
//! 2. Without the lock: the command itself runs on the snapshot, the same
//!    code [`Session::execute`] runs, so the result and the errors are the
//!    same for the same document.
//! 3. Under the lock again: if the document is still the active one at
//!    the same revision, with the same path and profiles, the result is
//!    stored (`DocState::adopt`): the same undo step, dirty flag,
//!    selection, last job and caches the command left. Otherwise nothing
//!    is stored, and the command runs once more on a new snapshot; when
//!    the document changed again it fails with [`changed_error`].
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
use crate::commands::project::{OpenParams, install_open, load_open};
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
    fn of(s: &Session) -> Self {
        let st = s.doc();
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
    let Some(what) = stale_policy(id) else {
        return lock(session).execute(id, params, origin);
    };
    match id {
        "project.open" => open(session, params, origin, what),
        "document.export" => split(
            session,
            id,
            origin,
            what,
            |scratch| {
                let p: ExportParams = crate::commands::parse(params.clone())?;
                prepare_export(scratch, p, origin)
            },
            PreparedExport::write,
        ),
        _ => split(
            session,
            id,
            origin,
            what,
            |scratch| scratch.execute(id, params.clone(), origin),
            Ok,
        ),
    }
}

/// The three steps of the module doc: `run` on a snapshot without the
/// lock, the check and the store under it, then `finish` (a file write)
/// without it again, only for a result that was stored.
fn split<T>(
    session: &Mutex<Session>,
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
            (
                s.scratch(),
                Key::of(&s),
                s.doc().twin_base(),
                s.unlocked_hook.clone(),
            )
        };
        let result = compute(id, hook, || run(&mut scratch));
        {
            let mut s = lock(session);
            let fresh = Key::of(&s) == key;
            if fresh {
                let twin = scratch
                    .docs
                    .pop()
                    .expect("a scratch session has one document");
                s.doc_mut().adopt(&base, twin);
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

/// `project.open` off the lock: the file is read and its shapes rebuilt
/// without it, then the document is opened under it, with the bookkeeping
/// of every command that changes the session (a panic rolls back, linked
/// copies are checked).
fn open(
    session: &Mutex<Session>,
    params: Value,
    origin: Origin,
    what: &str,
) -> Result<Value, CommandError> {
    let id = "project.open";
    for _ in 0..TRIES {
        let (p, profiles, hook) = {
            let mut s = lock(session);
            begin(&mut s, origin);
            let p: OpenParams = crate::commands::parse(params.clone())?;
            (p, s.profiles.clone(), s.unlocked_hook.clone())
        };
        let loaded = compute(id, hook, || load_open(&profiles, &p.path));
        let mut s = lock(session);
        if s.profiles.revision() != profiles.revision() {
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
        let install = Install {
            opened: Mutex::new(Some((p.path, document, warnings))),
        };
        return s.run_prepared(&install, origin);
    }
    Err(changed_error(what))
}

/// A project read and rebuilt off the lock, opened as the command
/// `project.open` (its id, so its errors read the same).
struct Install {
    opened: Mutex<Option<(PathBuf, Document, Vec<String>)>>,
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
