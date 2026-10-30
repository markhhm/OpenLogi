//! Press-owned, click-through effects around the native pointer.

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, BoxShadow, Context, DisplayId, IntoElement,
    ParentElement, Pixels, Point, Render, Styled, Window, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, div, point, px, size,
};
use openlogi_ipc::HighlightStyle;
use tracing::warn;

#[cfg(target_os = "macos")]
#[path = "highlight/macos.rs"]
mod native;
#[cfg(not(target_os = "macos"))]
#[path = "highlight/unsupported.rs"]
mod native;

// The effect uses logical screen geometry and black dimming, independent of UI theme/zoom.
const RADIUS: f32 = 120.0;
const DIM: gpui::Hsla = gpui::Hsla {
    h: 0.0,
    s: 0.0,
    l: 0.0,
    a: 0.6,
};

// Literal red and screen-space sizing are the presentation effect, independent of UI theme.
const LASER_RADIUS: f32 = 11.0;
const LASER: gpui::Hsla = gpui::Hsla {
    h: 0.0,
    s: 1.0,
    l: 0.55,
    a: 1.0,
};

#[derive(PartialEq)]
struct Frame {
    display_id: DisplayId,
    bounds: Bounds<Pixels>,
    pointer: Point<Pixels>,
}

/// The border is wider than the display diagonal; its transparent centre has radius RADIUS.
fn mask_geometry(frame: &Frame) -> (Bounds<Pixels>, Pixels) {
    let width = f32::from(frame.bounds.size.width);
    let height = f32::from(frame.bounds.size.height);
    let border = px(width.hypot(height) + 1.0);
    let outer_radius = border + px(RADIUS);
    let centre = frame.pointer - frame.bounds.origin;
    (
        Bounds::new(
            centre - point(outer_radius, outer_radius),
            size(outer_radius * 2.0, outer_radius * 2.0),
        ),
        border,
    )
}

struct HighlightView {
    frame: Frame,
    style: HighlightStyle,
}

impl Render for HighlightView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let effect = match self.style {
            HighlightStyle::Spotlight => {
                let (bounds, border) = mask_geometry(&self.frame);
                div()
                    .absolute()
                    .left(bounds.origin.x)
                    .top(bounds.origin.y)
                    .size(bounds.size.width)
                    .rounded_full()
                    .border(border)
                    .border_color(DIM)
            }
            HighlightStyle::Laser => {
                let centre = self.frame.pointer - self.frame.bounds.origin;
                div()
                    .absolute()
                    .left(centre.x - px(LASER_RADIUS))
                    .top(centre.y - px(LASER_RADIUS))
                    .size(px(LASER_RADIUS * 2.0))
                    .rounded_full()
                    .bg(LASER)
                    .shadow(vec![
                        BoxShadow::new(px(0.0), px(0.0), LASER.opacity(0.5))
                            .blur_radius(px(12.0))
                            .spread_radius(px(3.0)),
                    ])
            }
        };
        div().size_full().overflow_hidden().child(effect)
    }
}

impl HighlightView {
    fn next_frame(
        window: &mut Window,
        cx: &mut Context<Self>,
        mut capture: impl FnMut() -> Option<Frame> + 'static,
    ) {
        let view = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |view, cx| {
                if let Some(frame) = capture()
                    && frame != view.frame
                {
                    if frame.display_id != view.frame.display_id
                        || frame.bounds != view.frame.bounds
                    {
                        if let Err(error) = native::place(window, &frame) {
                            warn!(%error, "could not move the pointer highlight");
                            window.remove_window();
                            return;
                        }
                        window.bounds_changed(cx);
                    }
                    view.frame = frame;
                    cx.notify();
                }
                Self::next_frame(window, cx, capture);
            });
        });
    }
}

/// Show one highlight window, or close it as soon as the press or agent ends.
pub(crate) fn present(cx: &mut App, shown: Option<HighlightStyle>) {
    let open: Vec<WindowHandle<HighlightView>> = cx
        .windows()
        .iter()
        .filter_map(AnyWindowHandle::downcast::<HighlightView>)
        .collect();
    let Some(style) = shown else {
        for view in open {
            let _ = view.update(cx, |_, window, _| window.remove_window());
        }
        return;
    };
    if !open.is_empty() {
        for view in open {
            let _ = view.update(cx, |view, _, cx| {
                view.style = style;
                cx.notify();
            });
        }
        return;
    }
    let Some(frame) = native::capture() else {
        return;
    };
    let options = window_options(&frame);
    match cx.open_window(options, |window, cx| {
        cx.new(|cx| {
            HighlightView::next_frame(window, cx, native::capture);
            HighlightView { frame, style }
        })
    }) {
        Ok(opened) => {
            let _ = opened.update(cx, |view, window, cx| {
                if let Err(error) = native::place(window, &view.frame) {
                    warn!(%error, "could not place the pointer highlight");
                    window.remove_window();
                    return;
                }
                window.bounds_changed(cx);
            });
        }
        Err(error) => warn!(%error, "could not open the pointer highlight"),
    }
}

fn window_options(frame: &Frame) -> WindowOptions {
    WindowOptions {
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("openlogi-highlight".into()),
        display_id: Some(frame.display_id),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Point::default(),
            frame.bounds.size,
        ))),
        ..WindowOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame {
            display_id: DisplayId::from(1),
            bounds: Bounds::new(point(px(-1920.0), px(200.0)), size(px(1920.0), px(1080.0))),
            pointer: point(px(-1920.0), px(200.0)),
        }
    }

    #[test]
    fn highlight_mask_keeps_its_hole_at_the_pointer_and_covers_every_corner() {
        let mut frame = frame();
        for (x, y) in [
            (0.0, 0.0),
            (1920.0, 0.0),
            (0.0, 1080.0),
            (1920.0, 1080.0),
            (960.0, 540.0),
        ] {
            frame.pointer = frame.bounds.origin + point(px(x), px(y));
            let (mask, border) = mask_geometry(&frame);
            assert_eq!(mask.center(), point(px(x), px(y)));
            assert_eq!(mask.size.width / 2.0 - border, px(RADIUS));
            let radius = f32::from(mask.size.width) / 2.0;
            for (cx, cy) in [
                (0.0_f32, 0.0_f32),
                (1920.0, 0.0),
                (0.0, 1080.0),
                (1920.0, 1080.0),
            ] {
                assert!((cx - x).hypot(cy - y) < radius);
            }
        }
    }

    #[gpui::test]
    fn highlight_paints_one_transparent_hole_with_a_dim_border(cx: &mut gpui::TestAppContext) {
        let handle = cx.update(|cx| {
            let mut frame = frame();
            frame.pointer = frame.bounds.center();
            let options = window_options(&frame);
            assert!(!options.focus, "the highlight must not take keyboard focus");
            assert_eq!(options.kind, WindowKind::PopUp);
            assert_eq!(
                options.window_background,
                WindowBackgroundAppearance::Transparent
            );
            cx.open_window(options, |_, cx| {
                cx.new(|_| HighlightView {
                    frame,
                    style: HighlightStyle::Spotlight,
                })
            })
            .unwrap()
        });
        let mut visual = gpui::VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
            let quads = window.painted_quads();
            assert_eq!(quads.len(), 1, "the effect paints exactly one quad");
            let quad = &quads[0];
            assert!(quad.background.is_transparent());
            assert_eq!(quad.border_color, DIM);
            let scale = window.scale_factor();
            let hole = (quad.corner_radii.top_left.0 - quad.border_widths.left.0) / scale;
            assert!((hole - RADIUS).abs() < 0.01, "inner radius is {hole}");
            assert_eq!(
                quad.bounds.center(),
                point(px(960.0), px(540.0)).scale(scale)
            );
            assert_eq!(
                quad.content_mask.bounds.size,
                size(px(1920.0), px(1080.0)).scale(scale)
            );
        });
    }

    #[gpui::test]
    fn hiding_the_highlight_closes_its_window(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.open_window(window_options(&frame()), |_, cx| {
                cx.new(|_| HighlightView {
                    frame: frame(),
                    style: HighlightStyle::Spotlight,
                })
            })
            .unwrap();
            assert_eq!(cx.windows().len(), 1);
            present(cx, None);
        });
        cx.run_until_parked();
        cx.update(|cx| assert!(cx.windows().is_empty()));
    }
    #[gpui::test]
    fn laser_reuses_the_window_and_paints_a_red_dot_without_dimming(cx: &mut gpui::TestAppContext) {
        let handle = cx.update(|cx| {
            let mut frame = frame();
            frame.pointer = frame.bounds.center();
            cx.open_window(window_options(&frame), |_, cx| {
                cx.new(|_| HighlightView {
                    frame,
                    style: HighlightStyle::Spotlight,
                })
            })
            .unwrap()
        });
        cx.update(|cx| {
            present(cx, Some(HighlightStyle::Laser));
            assert_eq!(cx.windows().len(), 1);
        });
        let mut visual = gpui::VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
            let quads = window.painted_quads();
            assert_eq!(
                quads.len(),
                1,
                "only the dot paints a quad; no display dimmer"
            );
            let quad = &quads[0];
            let scale = window.scale_factor();
            assert_eq!(quad.background.as_solid(), Some(LASER));
            assert_eq!(quad.bounds.size, size(px(22.0), px(22.0)).scale(scale));
            assert_eq!(
                quad.bounds.center(),
                point(px(960.0), px(540.0)).scale(scale)
            );
            assert!((quad.corner_radii.top_left.0 / scale - 11.0).abs() < 0.01);
            assert!(quad.border_widths.left.0.abs() < f32::EPSILON);
            assert_eq!(
                quad.content_mask.bounds.size,
                size(px(1920.0), px(1080.0)).scale(scale)
            );
        });
        cx.update(|cx| present(cx, None));
        cx.run_until_parked();
        cx.update(|cx| assert!(cx.windows().is_empty()));
    }
    #[gpui::test]
    fn stationary_frames_keep_sampling_without_notifying(cx: &mut gpui::TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        let pointer = Rc::new(Cell::new(frame().pointer));
        let samples = Rc::new(Cell::new(0));
        let handle = cx.update(|cx| {
            cx.open_window(window_options(&frame()), |window, cx| {
                cx.new(|cx| {
                    let pointer = Rc::clone(&pointer);
                    let samples = Rc::clone(&samples);
                    HighlightView::next_frame(window, cx, move || {
                        samples.set(samples.get() + 1);
                        Some(Frame {
                            pointer: pointer.get(),
                            ..frame()
                        })
                    });
                    HighlightView {
                        frame: frame(),
                        style: HighlightStyle::Spotlight,
                    }
                })
            })
            .unwrap()
        });
        let view = handle.update(cx, |_, _, cx| cx.entity()).unwrap();
        let notifications = Rc::new(Cell::new(0));
        let _subscription = cx.update(|cx| {
            let notifications = Rc::clone(&notifications);
            cx.observe(&view, move |_, _| {
                notifications.set(notifications.get() + 1);
            })
        });
        cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        for _ in 0..3 {
            assert_eq!(
                cx.update_window(handle.into(), |_, window, cx| window
                    .simulate_next_frame(cx))
                    .unwrap(),
                1
            );
        }
        assert_eq!(
            samples.get(),
            3,
            "clean windows keep polling through on_next_frame"
        );
        assert_eq!(
            notifications.get(),
            0,
            "stationary frames must not request a redraw"
        );
        pointer.set(pointer.get() + point(px(1.0), px(0.0)));
        assert_eq!(
            cx.update_window(handle.into(), |_, window, cx| window
                .simulate_next_frame(cx))
                .unwrap(),
            1
        );
        assert_eq!(notifications.get(), 1, "motion invalidates the view once");
        assert_eq!(
            handle.read_with(cx, |view, _| view.frame.pointer).unwrap(),
            pointer.get()
        );
        assert_eq!(
            cx.update_window(handle.into(), |_, window, cx| window
                .simulate_next_frame(cx))
                .unwrap(),
            1
        );
        assert_eq!(samples.get(), 5);
        assert_eq!(
            notifications.get(),
            1,
            "the following stationary sample stays clean"
        );
    }
}
