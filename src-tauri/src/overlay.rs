//! Region-mode layer stack. Esc backs out one layer at a time, matching Cursor Annotate's feel.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    #[default]
    Pin,
    Pen,
    Highlight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayLayer {
    /// Overlay is closed.
    #[default]
    Idle,
    /// Freeze-frame is up; drag a box.
    Selecting,
    /// A box is chosen; pins, pen, highlight.
    Annotating,
    /// Placing a numbered pin (click to drop, Esc cancels the pin).
    Pinning,
    /// A stroke is in progress.
    Drawing,
}

/// Pop one layer. Idle stays idle.
pub fn esc(layer: OverlayLayer) -> OverlayLayer {
    match layer {
        OverlayLayer::Drawing | OverlayLayer::Pinning => OverlayLayer::Annotating,
        OverlayLayer::Annotating => OverlayLayer::Selecting,
        OverlayLayer::Selecting | OverlayLayer::Idle => OverlayLayer::Idle,
    }
}

pub fn hint(layer: OverlayLayer, tool: Tool) -> &'static str {
    match (layer, tool) {
        (OverlayLayer::Idle, _) => "",
        (OverlayLayer::Selecting, _) => {
            "Drag a box · Esc cancel"
        }
        (OverlayLayer::Annotating, Tool::Pin) => {
            "Click to pin · P pen · H highlight · Enter capture · Esc back"
        }
        (OverlayLayer::Annotating, Tool::Pen) => {
            "Draw · P pin · H highlight · Enter capture · Esc back"
        }
        (OverlayLayer::Annotating, Tool::Highlight) => {
            "Highlight · P pin · D pen · Enter capture · Esc back"
        }
        (OverlayLayer::Pinning, _) => "Click to drop pin · Esc cancel pin",
        (OverlayLayer::Drawing, _) => "Release to finish stroke · Esc cancel stroke",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_backs_out_one_layer_at_a_time() {
        assert_eq!(esc(OverlayLayer::Drawing), OverlayLayer::Annotating);
        assert_eq!(esc(OverlayLayer::Pinning), OverlayLayer::Annotating);
        assert_eq!(esc(OverlayLayer::Annotating), OverlayLayer::Selecting);
        assert_eq!(esc(OverlayLayer::Selecting), OverlayLayer::Idle);
        assert_eq!(esc(OverlayLayer::Idle), OverlayLayer::Idle);
        // A full unwind from a stroke.
        let mut l = OverlayLayer::Drawing;
        l = esc(l);
        l = esc(l);
        l = esc(l);
        assert_eq!(l, OverlayLayer::Idle);
    }

    #[test]
    fn the_hint_bar_names_the_gestures_for_each_layer() {
        assert!(hint(OverlayLayer::Selecting, Tool::Pin).contains("Drag"));
        assert!(hint(OverlayLayer::Annotating, Tool::Pin).contains("pin"));
        assert!(hint(OverlayLayer::Pinning, Tool::Pin).contains("Esc"));
        assert!(hint(OverlayLayer::Idle, Tool::Pin).is_empty());
    }
}
