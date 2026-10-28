//! Shared pointer/gesture geometry for V3 actions and workflows.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn center(self) -> Option<Point> {
        (self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.x.abs() <= 10_000_000.0
            && self.y.abs() <= 10_000_000.0
            && self.width > 0.0
            && self.height > 0.0
            && self.width <= 1_000_000.0
            && self.height <= 1_000_000.0)
            .then_some(Point {
                x: self.x + self.width / 2.0,
                y: self.y + self.height / 2.0,
            })
    }

    pub fn visible_within(self, viewport: Rect) -> bool {
        self.x >= viewport.x
            && self.y >= viewport.y
            && self.x + self.width <= viewport.x + viewport.width
            && self.y + self.height <= viewport.y + viewport.height
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerBackend {
    SemanticActivation,
    CdpMouse,
    FrameLocalCdpMouse,
    VisualGroundedCoordinate,
    KeyboardActivation,
}

pub fn click_backend_ladder(
    semantic_equivalent: bool,
    trusted_required: bool,
    frame_local: bool,
    uniquely_grounded: bool,
) -> Vec<PointerBackend> {
    let mut ladder = Vec::new();
    if semantic_equivalent && !trusted_required {
        ladder.push(PointerBackend::SemanticActivation);
    }
    ladder.push(if frame_local {
        PointerBackend::FrameLocalCdpMouse
    } else {
        PointerBackend::CdpMouse
    });
    if !uniquely_grounded {
        ladder.push(PointerBackend::VisualGroundedCoordinate);
    }
    if semantic_equivalent {
        ladder.push(PointerBackend::KeyboardActivation);
    }
    ladder
}

pub fn interpolate_path(from: Point, to: Point, steps: usize) -> Vec<Point> {
    let count = steps.clamp(2, 24);
    (1..=count)
        .map(|index| {
            let fraction = index as f64 / count as f64;
            Point {
                x: from.x + (to.x - from.x) * fraction,
                y: from.y + (to.y - from.y) * fraction,
            }
        })
        .collect()
}

pub fn semantic_rect(item: &serde_json::Value) -> Option<Rect> {
    let bounds = item.get("bounds")?;
    Some(Rect {
        x: bounds.get("x")?.as_f64()?,
        y: bounds.get("y")?.as_f64()?,
        width: bounds.get("width")?.as_f64()?,
        height: bounds.get("height")?.as_f64()?,
    })
}

pub fn semantic_item_center(item: &serde_json::Value) -> Option<(f64, f64)> {
    semantic_rect(item)
        .and_then(Rect::center)
        .map(|point| (point.x, point.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_and_visibility_are_bounded() {
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        };
        assert_eq!(rect.center(), Some(Point { x: 25.0, y: 40.0 }));
        assert!(rect.visible_within(Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0
        }));
        assert!(!rect.visible_within(Rect {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 100.0
        }));
        assert!(
            Rect {
                x: 1.0,
                y: 1.0,
                width: 0.0,
                height: 1.0
            }
            .center()
            .is_none()
        );
    }

    #[test]
    fn ladder_prefers_semantics_then_frame_local_mouse() {
        assert_eq!(
            click_backend_ladder(true, false, true, true),
            vec![
                PointerBackend::SemanticActivation,
                PointerBackend::FrameLocalCdpMouse,
                PointerBackend::KeyboardActivation
            ]
        );
        assert_eq!(
            click_backend_ladder(false, true, false, false),
            vec![
                PointerBackend::CdpMouse,
                PointerBackend::VisualGroundedCoordinate
            ]
        );
    }

    #[test]
    fn drag_paths_are_bounded_and_monotonic() {
        let path = interpolate_path(Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 20.0 }, 4);
        assert_eq!(path.len(), 4);
        assert_eq!(path.last(), Some(&Point { x: 10.0, y: 20.0 }));
        assert!(path.windows(2).all(|pair| pair[0].x <= pair[1].x));
        assert_eq!(
            interpolate_path(Point { x: 0.0, y: 0.0 }, Point { x: 1.0, y: 1.0 }, 100).len(),
            24
        );
    }
}
