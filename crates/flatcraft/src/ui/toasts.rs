//! Short messages in the corner of the window. Each one expires after 4 s.

use std::time::{Duration, Instant};

/// How long a toast stays visible.
pub const LIFETIME: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub created: Instant,
}

impl Toast {
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(text, ToastKind::Info)
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self::new(text, ToastKind::Warning)
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::new(text, ToastKind::Error)
    }

    fn new(text: impl Into<String>, kind: ToastKind) -> Self {
        Self {
            text: text.into(),
            kind,
            created: Instant::now(),
        }
    }

    pub fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.created) >= LIFETIME
    }
}

/// Removes the expired toasts and returns how long until the next one
/// expires, if any are left.
pub fn prune(toasts: &mut Vec<Toast>, now: Instant) -> Option<Duration> {
    toasts.retain(|t| !t.expired(now));
    toasts
        .iter()
        .map(|t| LIFETIME.saturating_sub(now.saturating_duration_since(t.created)))
        .min()
}

/// Draws the toasts in the bottom-right corner.
pub fn show(ctx: &egui::Context, toasts: &[Toast], tokens: &super::theme::Tokens) {
    if toasts.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::unique("flatcraft-toasts"))
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            egui::vec2(-tokens.spacing * 2.0, -tokens.spacing * 2.0),
        )
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            for t in toasts {
                let stroke = match t.kind {
                    ToastKind::Info => tokens.accent_prepare,
                    ToastKind::Warning => tokens.accent_cut,
                    ToastKind::Error => tokens.danger,
                };
                egui::Frame::new()
                    .fill(tokens.surface)
                    .stroke(egui::Stroke::new(1.0, stroke))
                    .corner_radius(tokens.radius)
                    .inner_margin(tokens.spacing)
                    .show(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.label(&t.text);
                    });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toasts_expire_after_four_seconds() {
        let mut toasts = vec![Toast::info("saved")];
        let t0 = toasts[0].created;
        assert_eq!(
            prune(&mut toasts, t0 + Duration::from_millis(3900)),
            Some(Duration::from_millis(100))
        );
        assert_eq!(toasts.len(), 1);
        assert_eq!(prune(&mut toasts, t0 + LIFETIME), None);
        assert!(toasts.is_empty());
    }
}
