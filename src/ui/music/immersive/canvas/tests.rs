use super::*;

const PALETTE: ((f64, f64, f64), (f64, f64, f64)) = ((0.2, 0.4, 0.6), (0.7, 0.5, 0.3));
const STYLES: [VisualizerStyle; 3] = [
    VisualizerStyle::Ribbons,
    VisualizerStyle::Tunnel,
    VisualizerStyle::Cosmic,
];

fn assert_rect(bounds: &graphene::Rect) {
    assert!(bounds.x().is_finite());
    assert!(bounds.y().is_finite());
    assert!(bounds.width().is_finite() && bounds.width() >= 0.0);
    assert!(bounds.height().is_finite() && bounds.height() >= 0.0);
    assert!((bounds.x() + bounds.width()).is_finite());
    assert!((bounds.y() + bounds.height()).is_finite());
}

fn assert_rounded_rect(outline: &gsk::RoundedRect) {
    assert_rect(outline.bounds());
    for corner in outline.corner() {
        assert!(corner.width().is_finite() && corner.width() >= 0.0);
        assert!(corner.height().is_finite() && corner.height() >= 0.0);
    }
}

fn assert_color(color: gdk::RGBA, palette: ((f64, f64, f64), (f64, f64, f64))) {
    for component in [color.red(), color.green(), color.blue(), color.alpha()] {
        assert!(component.is_finite() && (0.0..=1.0).contains(&component));
    }
    for (actual, accent, highlight) in [
        (color.red(), palette.0.0, palette.1.0),
        (color.green(), palette.0.1, palette.1.1),
        (color.blue(), palette.0.2, palette.1.2),
    ] {
        assert!(f64::from(actual) >= accent.min(highlight) - 0.00001);
        assert!(f64::from(actual) <= accent.max(highlight) + 0.00001);
    }
}

fn inspect(node: &gsk::RenderNode, palette: ((f64, f64, f64), (f64, f64, f64))) -> usize {
    assert!(!node.is::<gsk::CairoNode>(), "Cairo node in native frame");
    assert_rect(&node.bounds());
    if let Some(container) = node.downcast_ref::<gsk::ContainerNode>() {
        1 + (0..container.n_children())
            .map(|index| inspect(&container.child(index), palette))
            .sum::<usize>()
    } else if let Some(clip) = node.downcast_ref::<gsk::ClipNode>() {
        assert_rect(&clip.clip());
        1 + inspect(&clip.child(), palette)
    } else if let Some(clip) = node.downcast_ref::<gsk::RoundedClipNode>() {
        assert_rounded_rect(&clip.clip());
        1 + inspect(&clip.child(), palette)
    } else if let Some(transform) = node.downcast_ref::<gsk::TransformNode>() {
        assert!(
            transform
                .transform()
                .to_matrix()
                .to_float()
                .iter()
                .all(|value| value.is_finite())
        );
        1 + inspect(&transform.child(), palette)
    } else if let Some(color) = node.downcast_ref::<gsk::ColorNode>() {
        assert_color(color.color(), palette);
        1
    } else if let Some(border) = node.downcast_ref::<gsk::BorderNode>() {
        assert_rounded_rect(&border.outline());
        for width in border.widths() {
            assert!(width.is_finite() && *width >= 0.0);
        }
        for color in border.colors() {
            assert_color(*color, palette);
        }
        1
    } else {
        panic!("Unexpected native frame node: {:?}", node.node_type());
    }
}

fn assert_clipped(node: &gsk::RenderNode, width: f64, height: f64) {
    let bounds = node.bounds();
    assert!(bounds.x() >= -0.001 && bounds.y() >= -0.001);
    assert!(bounds.x() + bounds.width() <= width as f32 + 0.001);
    assert!(bounds.y() + bounds.height() <= height as f32 + 0.001);
}

fn node_limit(style: VisualizerStyle, width: f64) -> usize {
    match style {
        VisualizerStyle::Ribbons => {
            2 + 8 * ((width / 20.0).ceil() as usize).clamp(1, MAX_RIBBON_SEGMENTS)
        }
        VisualizerStyle::Tunnel => 10,
        VisualizerStyle::Cosmic => 66,
    }
}

fn frames_have_bounded_native_nodes() {
    for (width, height) in [
        (1.0, 1.0),
        (8.0, 2160.0),
        (48.0, 320.0),
        (320.0, 48.0),
        (1280.0, 720.0),
        (1920.0, 1080.0),
        (3840.0, 2160.0),
        (7680.0, 4320.0),
        (16384.0, 16384.0),
    ] {
        for style in STYLES {
            for time in [0.0, 0.125, 1.25, -10.5, f64::MAX] {
                for energy in [0.0, 0.5, 1.0] {
                    let snapshot = gtk::Snapshot::new();
                    snapshot_frame(&snapshot, width, height, style, time, [energy; 32], PALETTE);
                    let Some(node) = snapshot.to_node() else {
                        assert!(width <= 48.0 || height <= 48.0);
                        continue;
                    };
                    assert_clipped(&node, width, height);
                    let count = inspect(&node, PALETTE);
                    assert!(
                        count <= node_limit(style, width),
                        "{style:?} {width}x{height}: {count}"
                    );
                }
            }
        }
    }
}

fn invalid_frames_are_sanitized_and_empty_allocations_skipped() {
    let mut bands = [f32::NAN; 32];
    for (index, band) in bands.iter_mut().enumerate() {
        *band = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, f32::MAX][index % 5];
    }
    let palette = (
        (f64::NAN, f64::INFINITY, -1.0),
        (f64::NEG_INFINITY, 2.0, f64::MAX),
    );
    for style in STYLES {
        for time in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -f64::MAX] {
            let frame = Frame::new(style, time, bands, palette);
            assert!(frame.time.is_finite());
            assert!(
                frame
                    .bands
                    .iter()
                    .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
            );
            let snapshot = gtk::Snapshot::new();
            snapshot_frame(&snapshot, 3840.0, 2160.0, style, time, bands, palette);
            let node = snapshot.to_node().expect("4K sanitized frame");
            assert_clipped(&node, 3840.0, 2160.0);
            assert!(inspect(&node, frame.palette) <= node_limit(style, 3840.0));
        }
        for invalid in [
            0.0,
            -1.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MAX,
            f64::MIN_POSITIVE,
        ] {
            for (width, height) in [(invalid, 720.0), (1280.0, invalid)] {
                let snapshot = gtk::Snapshot::new();
                snapshot_frame(&snapshot, width, height, style, 1.0, bands, palette);
                assert!(snapshot.to_node().is_none());
            }
        }
    }
}

fn snapshot_state_is_balanced_and_palette_is_live() {
    let alternate = ((0.8, 0.2, 0.4), (0.4, 0.9, 0.7));
    for palette in [PALETTE, alternate] {
        for style in STYLES {
            let snapshot = gtk::Snapshot::new();
            snapshot_frame(&snapshot, 640.0, 360.0, style, 1.25, [0.5; 32], palette);
            let node = snapshot.to_node().expect("Native themed frame");
            inspect(&node, palette);

            let snapshot = gtk::Snapshot::new();
            snapshot_frame(&snapshot, 640.0, 360.0, style, 1.25, [0.5; 32], palette);
            snapshot.append_color(
                &gdk::RGBA::new(0.8, 0.2, 0.4, 1.0),
                &graphene::Rect::new(700.0, 500.0, 10.0, 10.0),
            );
            let node = snapshot.to_node().expect("Frame and sentinel");
            let bounds = node.bounds();
            assert!((bounds.x() + bounds.width() - 710.0).abs() < 0.01);
            assert!((bounds.y() + bounds.height() - 510.0).abs() < 0.01);
        }
    }
}

fn widget_has_normal_allocation_and_releases_without_callbacks() {
    for _ in 0..8 {
        let canvas = VisualizerCanvas::new();
        assert!(canvas.imp().frame.get().is_none());
        assert!(canvas.hexpands() && canvas.vexpands());
        let snapshot = gtk::Snapshot::new();
        WidgetImpl::snapshot(canvas.imp(), &snapshot);
        assert!(snapshot.to_node().is_none());
        let weak = canvas.downgrade();
        let horizontal = canvas.measure(gtk::Orientation::Horizontal, -1);
        let vertical = canvas.measure(gtk::Orientation::Vertical, -1);
        for style in STYLES {
            canvas.set_frame(style, 1.25, [0.5; 32], PALETTE);
            canvas.allocate(640, 360, -1, None);
            assert_eq!((canvas.width(), canvas.height()), (640, 360));
            assert_eq!(canvas.measure(gtk::Orientation::Horizontal, -1), horizontal);
            assert_eq!(canvas.measure(gtk::Orientation::Vertical, -1), vertical);
            let snapshot = gtk::Snapshot::new();
            WidgetImpl::snapshot(canvas.imp(), &snapshot);
            let node = snapshot.to_node().expect("Allocated widget snapshot");
            assert_clipped(&node, 640.0, 360.0);
            assert!(inspect(&node, PALETTE) <= node_limit(style, 640.0));
            assert!(!canvas.is_realized() && !canvas.is_mapped());
            canvas.clear();
            canvas.clear();
            assert!(canvas.imp().frame.get().is_none());
            let snapshot = gtk::Snapshot::new();
            WidgetImpl::snapshot(canvas.imp(), &snapshot);
            assert!(snapshot.to_node().is_none());
        }
        drop(canvas);
        assert!(weak.upgrade().is_none());
    }
}

#[test]
#[ignore = "Requires an isolated GTK display; run alone on one test thread"]
fn native_gsk_frames_and_widget_lifecycle() {
    gtk::init().expect("Isolated GTK display available");
    frames_have_bounded_native_nodes();
    invalid_frames_are_sanitized_and_empty_allocations_skipped();
    snapshot_state_is_balanced_and_palette_is_live();
    widget_has_normal_allocation_and_releases_without_callbacks();
}
