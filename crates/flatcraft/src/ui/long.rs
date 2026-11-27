//! The window's long commands (import, trace, reload, relink, open,
//! export) on a worker thread, through the engine's unlocked path
//! (`flatcraft_engine::unlocked`): the session lock is held only for the
//! snapshot and the store, so the window keeps drawing and an agent keeps
//! working. One runs at a time, in the order they were asked for, so a
//! drop of files and projects keeps its order.
//!
//! A frame waits a moment ([`LONG_WAIT`]) for a command it just started,
//! so a small import is done at once; a long one finishes in a later
//! frame. Meanwhile the app bar shows a busy note ([`LongJobs::note`])
//! and the actions that would start another long command are dimmed,
//! with a card that says why ([`BUSY_CARD`]).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use flatcraft_engine::CommandError;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::Value;

/// How long a frame waits for a long command it just started.
pub const LONG_WAIT: Duration = Duration::from_millis(50);

/// Why an action that starts a long command is dimmed while one runs.
pub const BUSY_CARD: &str = "Wait a moment: FlatCraft is still busy with the last import, trace, file or export. The note at the top right says what it is doing. This one is ready again when that is done.";

/// The hover card of the busy note.
pub const NOTE_CARD: &str = "FlatCraft is working on this in the background. The window keeps working meanwhile: you can look around, zoom and change things. Imports, traces, opening and exporting you ask for now wait their turn, and the note says what comes next.";

/// What a long command works on and runs.
#[derive(Debug, Clone)]
pub enum Work {
    /// A registry command in the document `doc_id` while it is active;
    /// `None` is the document active when the command begins.
    Command {
        doc_id: Option<u64>,
        command: &'static str,
        params: Value,
    },
    /// `project.open` of a file.
    Open(PathBuf),
    /// A sample project built into the window.
    Sample { bytes: Arc<[u8]>, title: String },
}

/// What the window does with the result.
#[derive(Debug, Clone, PartialEq)]
pub enum Then {
    /// `after_command` while `doc_id` is still active.
    Command { doc_id: Option<u64> },
    /// The same, then Prepare (a drop of files).
    ImportToPrepare,
    /// The "Exported" toast for `path`.
    Export(PathBuf),
    /// Recent list, thumbnail, warnings and Prepare; `replaces` is a
    /// recent entry the file was located for.
    Open {
        path: PathBuf,
        replaces: Option<PathBuf>,
    },
    /// The warnings and Prepare.
    Sample,
}

/// A long command and what follows it.
#[derive(Debug, Clone)]
pub struct LongJob {
    pub work: Work,
    pub then: Then,
}

impl LongJob {
    /// The registry command it runs.
    pub fn command(&self) -> &'static str {
        match &self.work {
            Work::Command { command, .. } => command,
            Work::Open(_) | Work::Sample { .. } => "project.open",
        }
    }

    /// What the busy note says while it runs.
    pub fn note(&self) -> &'static str {
        note(self.command())
    }
}

/// The busy note of `command`.
pub fn note(command: &str) -> &'static str {
    match command {
        "object.import" | "object.import_svg" => "Importing...",
        "object.retrace" => "Tracing...",
        "object.reload" => "Reading the files again...",
        "object.relink" => "Reading the file...",
        "document.export" => "Exporting...",
        "project.open" => "Opening...",
        _ => "Working...",
    }
}

/// What the busy note says of a command that waits: "opening".
pub fn next_text(command: &str) -> &'static str {
    match command {
        "object.import" | "object.import_svg" => "importing",
        "object.retrace" => "tracing",
        "object.reload" => "reading the files again",
        "object.relink" => "reading the file",
        "document.export" => "exporting",
        "project.open" => "opening",
        _ => "more work",
    }
}

/// A finished long command: what it was and what it gave.
pub type Done = (LongJob, Result<Value, CommandError>);

/// The queue of long commands and the one that runs.
pub struct LongJobs {
    queue: VecDeque<LongJob>,
    running: Option<(LongJob, Receiver<Result<Value, CommandError>>)>,
    repaint: egui::Context,
}

impl LongJobs {
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            queue: VecDeque::new(),
            running: None,
            repaint: ctx.clone(),
        }
    }

    /// The busy note while a long command runs or waits: what runs, and
    /// what waits behind it ("Importing... (opening next)"), so a second
    /// request (an Open by Cmd+O) shows that it was taken.
    pub fn note(&self) -> Option<String> {
        let mut jobs = self.running.iter().map(|(j, _)| j).chain(self.queue.iter());
        let first = jobs.next()?;
        let waiting: Vec<&LongJob> = jobs.collect();
        Some(match waiting.as_slice() {
            [] => first.note().to_string(),
            [next] => format!("{} ({} next)", first.note(), next_text(next.command())),
            [next, rest @ ..] => format!(
                "{} ({} next, {} more waiting)",
                first.note(),
                next_text(next.command()),
                rest.len()
            ),
        })
    }

    /// True while a long command runs or waits.
    pub fn busy(&self) -> bool {
        self.running.is_some() || !self.queue.is_empty()
    }

    /// Queues `job`.
    pub fn push(&mut self, job: LongJob) {
        self.queue.push_back(job);
    }

    /// Starts the next command when none runs, and gives back the one that
    /// finished, if any: waits [`LONG_WAIT`] for one it just started. Call
    /// it again after handling a result, until it gives `None`.
    pub fn poll(&mut self, session: &SharedSession) -> Option<Done> {
        let mut started = false;
        if self.running.is_none() {
            let job = self.queue.pop_front()?;
            let (tx, rx) = mpsc::channel();
            let work = job.work.clone();
            let shared = session.clone();
            let repaint = self.repaint.clone();
            let spawned = std::thread::Builder::new()
                .name("flatcraft-long".into())
                .spawn(move || {
                    let _ = tx.send(run(&shared, work));
                    repaint.request_repaint();
                });
            if spawned.is_err() {
                // No thread: run it here rather than never.
                let result = run(session, job.work.clone());
                return Some((job, result));
            }
            self.running = Some((job, rx));
            started = true;
        }
        let (_, rx) = self.running.as_ref()?;
        let result = if started {
            rx.recv_timeout(LONG_WAIT).ok()
        } else {
            match rx.try_recv() {
                Ok(r) => Some(r),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(CommandError::Failed(
                    "FlatCraft hit an internal error, so this command changed nothing.".into(),
                ))),
            }
        }?;
        let (job, _) = self.running.take()?;
        Some((job, result))
    }
}

impl std::fmt::Debug for LongJobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongJobs")
            .field("queued", &self.queue.len())
            .field("running", &self.running.as_ref().map(|(j, _)| j.command()))
            .finish()
    }
}

/// Runs `work` as the person, off the session lock.
fn run(session: &SharedSession, work: Work) -> Result<Value, CommandError> {
    use flatcraft_engine::unlocked;
    match work {
        Work::Command {
            doc_id: Some(doc),
            command,
            params,
        } => unlocked::execute_in(session, doc, command, params, Origin::User),
        Work::Command {
            doc_id: None,
            command,
            params,
        } => unlocked::execute(session, command, params, Origin::User),
        Work::Open(path) => unlocked::execute(
            session,
            "project.open",
            serde_json::json!({ "path": path }),
            Origin::User,
        ),
        Work::Sample { bytes, title } => unlocked::open_bytes(session, bytes, &title, Origin::User),
    }
}
