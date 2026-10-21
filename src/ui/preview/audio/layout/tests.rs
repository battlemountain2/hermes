use super::*;

#[test]
fn wide_layout_requires_enough_room_for_both_children() {
    assert!(!is_wide(499, 230, 220));
    assert!(is_wide(500, 230, 220));
    assert!(!is_wide(600, 230, 450));
    assert!(is_wide(700, 230, 450));
    assert!(!is_wide(-1, 230, 220));
    assert!(!is_wide(i32::MAX - 1, i32::MAX, 220));
}

#[test]
#[ignore = "Private GTK display; exercises real height-for-width allocation without a media decoder"]
fn desktop_player_card_reflows_without_a_width_property_or_resize_loop() {
    gtk::init().unwrap();
    let hero = gtk::DrawingArea::new();
    hero.set_content_width(230);
    hero.set_content_height(180);
    let controls = gtk::DrawingArea::new();
    controls.set_content_width(220);
    controls.set_content_height(200);
    let card = PlayerCard::new(&hero, &controls);
    assert_eq!(card.request_mode(), gtk::SizeRequestMode::HeightForWidth);
    for width in [320, 600, 320, 600] {
        let (minimum, natural, _, _) = card.measure(gtk::Orientation::Vertical, width);
        assert_eq!(minimum, natural);
        card.allocate(width, natural, -1, None);
        let hero_bounds = hero.compute_bounds(&card).unwrap();
        let controls_bounds = controls.compute_bounds(&card).unwrap();
        if width >= WIDE_THRESHOLD {
            assert_eq!(natural, 200);
            assert!(controls_bounds.x() >= hero_bounds.x() + hero_bounds.width() + SPACING as f32);
            assert_eq!(controls_bounds.y(), 0.0);
        } else {
            assert_eq!(natural, 396);
            assert!(controls_bounds.y() >= hero_bounds.y() + hero_bounds.height() + SPACING as f32);
        }
        assert!(controls_bounds.x() + controls_bounds.width() <= width as f32);
        assert!(controls_bounds.y() + controls_bounds.height() <= natural as f32);
        assert_eq!(card.measure(gtk::Orientation::Horizontal, -1).0, 230);
    }
    controls.set_content_width(450);
    assert_eq!(card.measure(gtk::Orientation::Vertical, 600).1, 396);
    assert_eq!(card.measure(gtk::Orientation::Vertical, 720).1, 200);
    controls.set_content_width(220);
    card.set_direction(gtk::TextDirection::Rtl);
    card.allocate(600, 200, -1, None);
    assert!(hero.compute_bounds(&card).unwrap().x() > controls.compute_bounds(&card).unwrap().x());
    let weak_hero = hero.downgrade();
    let weak_controls = controls.downgrade();
    drop(hero);
    drop(controls);
    drop(card);
    assert!(weak_hero.upgrade().is_none());
    assert!(weak_controls.upgrade().is_none());
}
