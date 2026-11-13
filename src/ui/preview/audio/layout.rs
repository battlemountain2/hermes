use gtk::{glib, prelude::*, subclass::prelude::*};

const SPACING: i32 = 16;
const WIDE_THRESHOLD: i32 = 500;

#[cfg(test)]
mod tests;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PlayerCard;

    #[glib::object_subclass]
    impl ObjectSubclass for PlayerCard {
        const NAME: &'static str = "HermesAudioPlayerCard";
        type Type = super::PlayerCard;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for PlayerCard {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for PlayerCard {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some((hero, controls)) = self.obj().parts() else {
                return (0, 0, -1, -1);
            };
            let (hero_min, hero_natural, _, _) = hero.measure(gtk::Orientation::Horizontal, -1);
            let (controls_min, controls_natural, _, _) =
                controls.measure(gtk::Orientation::Horizontal, -1);
            if orientation == gtk::Orientation::Horizontal {
                return (
                    hero_min.max(controls_min),
                    hero_natural.max(controls_natural),
                    -1,
                    -1,
                );
            }
            let wide = is_wide(for_size, hero_natural, controls_min);
            let hero_width = if wide { hero_natural } else { for_size };
            let controls_width = if wide {
                for_size - hero_natural - SPACING
            } else {
                for_size
            };
            let (hero_minimum, hero_height, _, _) =
                hero.measure(gtk::Orientation::Vertical, hero_width);
            let (controls_minimum, controls_height, _, _) =
                controls.measure(gtk::Orientation::Vertical, controls_width);
            if wide {
                (
                    hero_minimum.max(controls_minimum),
                    hero_height.max(controls_height),
                    -1,
                    -1,
                )
            } else {
                (
                    hero_minimum + controls_minimum + SPACING,
                    hero_height + controls_height + SPACING,
                    -1,
                    -1,
                )
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _: i32) {
            let Some((hero, controls)) = self.obj().parts() else {
                return;
            };
            let (_, hero_natural, _, _) = hero.measure(gtk::Orientation::Horizontal, -1);
            let (controls_min, _, _, _) = controls.measure(gtk::Orientation::Horizontal, -1);
            let hero_width = hero_natural.min(width).max(0);
            let (_, hero_height, _, _) = hero.measure(gtk::Orientation::Vertical, hero_width);
            let hero_height = hero_height.min(height).max(0);
            if is_wide(width, hero_natural, controls_min) {
                let controls_width = (width - hero_width - SPACING).max(0);
                let (_, controls_height, _, _) =
                    controls.measure(gtk::Orientation::Vertical, controls_width);
                let controls_height = controls_height.min(height).max(0);
                let right_to_left = self.obj().direction() == gtk::TextDirection::Rtl;
                let hero_start = if right_to_left { width - hero_width } else { 0 };
                let controls_start = if right_to_left {
                    0
                } else {
                    hero_width + SPACING
                };
                hero.size_allocate(
                    &gtk::Allocation::new(
                        hero_start,
                        (height - hero_height) / 2,
                        hero_width,
                        hero_height,
                    ),
                    -1,
                );
                controls.size_allocate(
                    &gtk::Allocation::new(
                        controls_start,
                        (height - controls_height) / 2,
                        controls_width,
                        controls_height,
                    ),
                    -1,
                );
            } else {
                hero.size_allocate(
                    &gtk::Allocation::new((width - hero_width) / 2, 0, hero_width, hero_height),
                    -1,
                );
                controls.size_allocate(
                    &gtk::Allocation::new(
                        0,
                        hero_height + SPACING,
                        width,
                        (height - hero_height - SPACING).max(0),
                    ),
                    -1,
                );
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some((hero, controls)) = self.obj().parts() {
                self.obj().snapshot_child(&hero, snapshot);
                self.obj().snapshot_child(&controls, snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct PlayerCard(ObjectSubclass<imp::PlayerCard>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PlayerCard {
    pub(super) fn new(hero: &impl IsA<gtk::Widget>, controls: &impl IsA<gtk::Widget>) -> Self {
        let card: Self = glib::Object::new();
        card.add_css_class("preview-audio-player-card");
        card.set_hexpand(true);
        hero.set_parent(&card);
        controls.set_parent(&card);
        card
    }

    fn parts(&self) -> Option<(gtk::Widget, gtk::Widget)> {
        Some((self.first_child()?, self.last_child()?))
    }
}

fn is_wide(width: i32, hero_width: i32, controls_min: i32) -> bool {
    width >= WIDE_THRESHOLD
        && width
            >= hero_width
                .saturating_add(controls_min)
                .saturating_add(SPACING)
}
