//! The Debug panel: the byte log of this session, the listing of the
//! current job, the device status and a viewer for capture fixtures.
//!
//! The byte log holds up to 100 000 entries, so only its tail is copied,
//! and only when it changed; decoding happens outside the session lock.

use std::path::Path;
use std::sync::Arc;

use egui::{RichText, Ui};
use flatcraft_device::capture::{CaptureRecord, read_jsonl};
use flatcraft_device::gpgl::decode;
use flatcraft_device::{Dir, TransportSpec};
use flatcraft_engine::JobResult;

use super::theme::Tokens;
use super::{Action, FlatCraftApp, fonts, lock};

/// Most byte log rows the panel shows (the newest ones).
pub const MAX_ROWS: usize = 5000;
/// Longest hex text in a row, in characters.
const HEX_CHARS: usize = 48;

/// One row of the byte log or capture table.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub time: String,
    pub dir: &'static str,
    pub hex: String,
    pub meaning: String,
}

/// Lower-case hex, shortened with "..." past [`HEX_CHARS`].
fn short_hex(bytes: &[u8]) -> String {
    let mut s = flatcraft_device::bytelog::hex(bytes);
    if s.len() > HEX_CHARS {
        s.truncate(HEX_CHARS);
        s.push_str("...");
    }
    s
}

/// What the bytes mean: the decoded commands going out, the reply text
/// coming in.
fn meaning(dir: Dir, bytes: &[u8]) -> String {
    match dir {
        Dir::Out => decode(bytes)
            .into_iter()
            .map(|c| c.meaning)
            .collect::<Vec<_>>()
            .join("; "),
        Dir::In => {
            let text: String = bytes
                .iter()
                .map(|&b| match b {
                    0x03 => String::new(),
                    0x20..=0x7e => char::from(b).to_string(),
                    _ => format!("\\x{b:02x}"),
                })
                .collect();
            format!("reply {text}")
        }
    }
}

pub fn row(t_s: f64, dir: Dir, bytes: &[u8]) -> Row {
    Row {
        time: format!("{t_s:.3} s"),
        dir: match dir {
            Dir::Out => "out",
            Dir::In => "in",
        },
        hex: short_hex(bytes),
        meaning: meaning(dir, bytes),
    }
}

/// The tab the panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebugTab {
    #[default]
    ByteLog,
    Listing,
    Capture,
}

impl DebugTab {
    const ALL: [DebugTab; 3] = [DebugTab::ByteLog, DebugTab::Listing, DebugTab::Capture];

    fn name(self) -> &'static str {
        match self {
            DebugTab::ByteLog => "Byte log",
            DebugTab::Listing => "Listing",
            DebugTab::Capture => "Capture",
        }
    }
}

/// Which listing the Listing tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListingView {
    /// The job the Cut workspace planned for the document as it is now.
    #[default]
    Preview,
    /// The job last sent from the active document (`DocState.last_job`).
    LastSent,
}

/// A capture fixture opened in the viewer.
#[derive(Debug, Clone, Default)]
pub struct Capture {
    pub name: String,
    pub rows: Vec<Row>,
}

/// What the Debug panel remembers between frames.
#[derive(Debug, Clone, Default)]
pub struct DebugState {
    pub tab: DebugTab,
    /// Length, last time and last size of the log the rows were made from.
    log_key: Option<(usize, u64, usize)>,
    pub log_rows: Vec<Row>,
    /// Entries of the log, including those not shown.
    pub log_len: usize,
    pub capture: Option<Capture>,
    pub listing: ListingView,
    /// The active document's last sent job, copied when it changed.
    pub last_sent: Option<Arc<JobResult>>,
}

impl DebugState {
    /// Copies the tail of the session's byte log when it changed and
    /// decodes it after the lock is released.
    pub fn refresh_log(&mut self, app_session: &flatcraft_mcp::SharedSession) {
        let tail = {
            let s = lock(app_session);
            let log = &s.byte_log;
            let key = log
                .entries()
                .last()
                .map(|e| (log.len(), e.t_ms, e.bytes.len()));
            if key == self.log_key {
                None
            } else {
                self.log_key = key;
                self.log_len = log.len();
                let skip = log.len().saturating_sub(MAX_ROWS);
                Some(log.entries().skip(skip).cloned().collect::<Vec<_>>())
            }
        };
        if let Some(tail) = tail {
            self.log_rows = tail
                .iter()
                .map(|e| row(e.t_ms as f64 / 1000.0, e.dir, &e.bytes))
                .collect();
        }
    }
}

impl DebugState {
    /// Copies the active document's last sent job when it changed. The
    /// check compares the bytes and the written files, without a copy.
    pub fn refresh_last_sent(&mut self, session: &flatcraft_mcp::SharedSession) {
        let s = lock(session);
        let now = s.doc().last_job.as_ref().filter(|j| j.sent.is_some());
        let same = match (now, &self.last_sent) {
            (Some(n), Some(c)) => n.sent == c.sent && n.job.bytes == c.job.bytes,
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.last_sent = now.cloned().map(Arc::new);
        }
    }
}

/// The listing lines of a job, as the `.gpgl.txt` file has them.
fn listing(ui: &mut Ui, job: &JobResult) {
    let lines = &job.job.listing;
    egui::ScrollArea::both()
        .id_salt("flatcraft-debug-listing")
        .auto_shrink([false, false])
        .show_rows(ui, 15.0, lines.len(), |ui, range| {
            for l in &lines[range] {
                ui.label(
                    RichText::new(format!("{}\t# {}", l.command, l.note))
                        .font(egui::FontId::monospace(11.0)),
                );
            }
        });
}

/// Reads a capture fixture (`.jsonl`, Task 19's format) into rows.
pub fn read_capture(path: &Path) -> Result<Capture, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let records: Vec<CaptureRecord> = read_jsonl(&text)?;
    Ok(Capture {
        name: super::display_name(path),
        rows: records.iter().map(|r| row(r.t, r.dir, &r.bytes)).collect(),
    })
}

/// The device line: M1 has no live device yet.
pub fn device_status(transport: Option<&TransportSpec>) -> &'static str {
    match transport {
        Some(TransportSpec::Usb { .. }) => "No device (USB cutting arrives in milestone 3)",
        _ => "No device (File Output)",
    }
}

fn table(ui: &mut Ui, id: &str, rows: &[Row], t: &Tokens) {
    if rows.is_empty() {
        ui.label(RichText::new("Nothing here yet.").color(t.text_dim));
        return;
    }
    let mono = egui::FontId::monospace(11.0);
    let height = ui.text_style_height(&egui::TextStyle::Body).max(14.0) + 4.0;
    ui.horizontal(|ui| {
        for (w, h) in [
            (70.0, "Time"),
            (30.0, "Dir"),
            (330.0, "Hex"),
            (0.0, "Meaning"),
        ] {
            let l = egui::Label::new(RichText::new(h).font(fonts::semibold(12.0)));
            if w > 0.0 {
                ui.add_sized([w, height], l);
            } else {
                ui.add(l);
            }
        }
    });
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, height, rows.len(), |ui, range| {
            for r in &rows[range] {
                ui.horizontal(|ui| {
                    ui.add_sized([70.0, height], egui::Label::new(&r.time));
                    ui.add_sized([30.0, height], egui::Label::new(r.dir));
                    ui.add_sized(
                        [330.0, height],
                        egui::Label::new(RichText::new(&r.hex).font(mono.clone())),
                    );
                    ui.label(&r.meaning);
                });
            }
        });
}

/// The Debug panel at the bottom-right of the window.
pub fn show(ctx: &egui::Context, app: &mut FlatCraftApp, actions: &mut Vec<Action>) {
    let t = app.tokens;
    let transport = app.ui.cut.transport.clone();
    let job = app.ui.cut.job();
    let mut open = app.ui.show_debug;
    let corner = ctx.content_rect().right_bottom() - egui::vec2(16.0, 16.0);
    let st = &mut app.ui.debug;
    egui::Window::new("Debug")
        .id(egui::Id::unique("flatcraft-debug"))
        .open(&mut open)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .default_pos(corner)
        .default_size([720.0, 320.0])
        .resizable(true)
        .collapsible(true)
        .show(ctx, |ui| {
            ui.label(RichText::new(device_status(transport.as_ref())).color(t.text_dim));
            ui.horizontal(|ui| {
                for tab in DebugTab::ALL {
                    let on = st.tab == tab;
                    let r = ui.add(egui::Button::selectable(on, tab.name()));
                    r.widget_info(|| {
                        egui::WidgetInfo::selected(egui::accesskit::Role::Tab, true, on, tab.name())
                    });
                    if r.clicked() {
                        st.tab = tab;
                    }
                }
            });
            ui.separator();
            match st.tab {
                DebugTab::ByteLog => {
                    if st.log_len > st.log_rows.len() {
                        ui.label(
                            RichText::new(format!(
                                "The newest {} of {} entries.",
                                st.log_rows.len(),
                                st.log_len
                            ))
                            .color(t.text_dim),
                        );
                    }
                    table(ui, "flatcraft-debug-log", &st.log_rows, &t);
                }
                DebugTab::Listing => {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut st.listing, ListingView::Preview, "Preview");
                        let has = st.last_sent.is_some();
                        let r = ui.add_enabled(
                            has,
                            egui::RadioButton::new(
                                st.listing == ListingView::LastSent,
                                "Last sent",
                            ),
                        );
                        if r.clicked() {
                            st.listing = ListingView::LastSent;
                        }
                    });
                    match (st.listing, &job, &st.last_sent) {
                        (ListingView::LastSent, _, Some(sent)) => {
                            let name = sent
                                .sent
                                .as_ref()
                                .and_then(|r| r.files.get(1))
                                .map_or_else(|| "the last job".into(), |f| super::display_name(f));
                            ui.label(RichText::new(format!("Last sent: {name}")).color(t.text_dim));
                            listing(ui, sent);
                        }
                        (_, Some(job), _) => {
                            ui.label(
                                RichText::new("Preview of the current job (.gpgl.txt)")
                                    .color(t.text_dim),
                            );
                            listing(ui, job);
                        }
                        _ => {
                            ui.label(
                                RichText::new("No job yet. Open Path in the Cut workspace.")
                                    .color(t.text_dim),
                            );
                        }
                    }
                }
                DebugTab::Capture => {
                    ui.horizontal(|ui| {
                        if ui
                            .button("Open capture...")
                            .on_hover_text("Load a .jsonl capture fixture and decode it.")
                            .clicked()
                        {
                            actions.push(Action::OpenCapture);
                        }
                        if let Some(c) = &st.capture {
                            ui.label(RichText::new(&c.name).color(t.text_dim));
                        }
                    });
                    match &st.capture {
                        Some(c) => table(ui, "flatcraft-debug-capture", &c.rows, &t),
                        None => {
                            ui.label(
                                RichText::new(
                                    "Load a capture to see what the plotter really heard.",
                                )
                                .color(t.text_dim),
                            );
                        }
                    }
                }
            }
        });
    app.ui.show_debug = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_decode_out_and_show_replies() {
        let r = row(1.5, Dir::Out, b"H\x03");
        assert_eq!(r.time, "1.500 s");
        assert_eq!(r.dir, "out");
        assert_eq!(r.hex, "4803");
        assert_eq!(r.meaning, "home");
        assert_eq!(row(0.0, Dir::In, b"0\x03").meaning, "reply 0");
        let long = row(0.0, Dir::Out, &[0x41; 40]);
        assert!(long.hex.ends_with("..."));
        assert_eq!(long.hex.len(), HEX_CHARS + 3);
    }
}
