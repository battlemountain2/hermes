use super::*;

#[test]
#[ignore = "Private GTK display; native offscreen backdrop nodes and synthetic pixels only"]
fn artwork_backdrop_has_bounded_native_nodes_and_no_idle_callback() {
    gtk::init().expect("initialize the isolated GTK display");
    let backdrop = ArtworkBackdrop::new();
    backdrop.allocate(1200, 900, -1, None);
    backdrop.set_base(gdk::RGBA::new(0.1, 0.1, 0.1, 1.0));
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 64, 64).expect("expected test value from gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 64, 64)");
    pixels.fill(0xee8020ff);
    backdrop.set_artwork(Some(gdk::Texture::for_pixbuf(&pixels)));
    let snapshot = gtk::Snapshot::new();
    WidgetImpl::snapshot(backdrop.imp(), &snapshot);
    let node = snapshot
        .to_node()
        .expect("expected test value from snapshot.to_node()");
    let container = node
        .downcast_ref::<gtk::gsk::ContainerNode>()
        .expect("expected test value from node.downcast_ref::<gtk::gsk::ContainerNode>()");
    assert_eq!(container.n_children(), 2);
    assert!(
        container
            .child(0)
            .downcast_ref::<gtk::gsk::ColorNode>()
            .is_some()
    );
    let overlay = container.child(1);
    let opacity = overlay
        .downcast_ref::<gtk::gsk::OpacityNode>()
        .expect("expected test value from overlay.downcast_ref::<gtk::gsk::OpacityNode>()");
    assert!((opacity.opacity() - super::super::artwork::BACKDROP_OPACITY).abs() < 0.0001);
    assert!(
        opacity
            .child()
            .downcast_ref::<gtk::gsk::TextureNode>()
            .is_some()
    );
    assert_eq!(node.bounds(), graphene::Rect::new(0.0, 0.0, 1200.0, 900.0));
    assert!(!backdrop.is_mapped() && !backdrop.is_realized());
    backdrop.set_artwork(None);
    let snapshot = gtk::Snapshot::new();
    WidgetImpl::snapshot(backdrop.imp(), &snapshot);
    assert!(
        snapshot
            .to_node()
            .expect("expected test value from snapshot .to_node()")
            .downcast_ref::<gtk::gsk::ColorNode>()
            .is_some()
    );
    let weak = backdrop.downgrade();
    drop(backdrop);
    assert!(weak.upgrade().is_none());
}
