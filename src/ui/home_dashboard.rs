// SPDX-License-Identifier: GPL-3.0-or-later

use std::{path::PathBuf, rc::Rc};

use gtk::{gio, glib, prelude::*};

use crate::{
    assets::{self, icons},
    model::Location,
    ui::{browser::BrowserView, formatting::format_timestamp},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DateBucket {
    Today,
    Yesterday,
    EarlierThisWeek,
    Older,
}

impl DateBucket {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::EarlierThisWeek => "Earlier this week",
            Self::Older => "Older",
        }
    }

    pub(crate) fn from_timestamp(seconds: i64, now: &glib::DateTime) -> Self {
        let Ok(date) = glib::DateTime::from_unix_utc(seconds)
            .and_then(|d| d.to_timezone(&now.timezone()))
        else {
            return Self::Older;
        };
        if same_day(&date, now) {
            Self::Today
        } else if now
            .add_days(-1)
            .is_ok_and(|yesterday| same_day(&date, &yesterday))
        {
            Self::Yesterday
        } else {
            let elapsed = now.to_unix().saturating_sub(seconds);
            if elapsed < 7 * 86400 {
                Self::EarlierThisWeek
            } else {
                Self::Older
            }
        }
    }
}

pub(crate) fn same_day(first: &glib::DateTime, second: &glib::DateTime) -> bool {
    first.year() == second.year() && first.day_of_year() == second.day_of_year()
}

pub(crate) struct RecentFileItem {
    pub uri: String,
    pub name: String,
    pub parent_path: String,
    pub parent_location: Location,
    pub modified_unix: i64,
    pub icon_name: &'static str,
}

pub(crate) fn query_recent_files(limit: usize) -> Vec<RecentFileItem> {
    let manager = gtk::RecentManager::default();
    let items = manager.items();
    let mut files = Vec::new();

    for item in items {
        if !item.exists() {
            continue;
        }
        let uri = item.uri().to_string();
        if uri.starts_with("trash://") || uri.starts_with("search://") || uri.starts_with("home://")
        {
            continue;
        }
        let gio_file = gio::File::for_uri(&uri);
        let Some(native_path) = gio_file.path() else {
            continue;
        };
        if native_path.is_dir() {
            continue;
        }
        let file_name = native_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| item.display_name().to_string());
        if file_name.starts_with('.') || file_name.ends_with('~') {
            continue;
        }

        let parent_location = native_path
            .parent()
            .map(|p| Location::local(p.to_path_buf()))
            .unwrap_or_else(|| Location::local(PathBuf::from("/")));
        let parent_path = parent_location.display_path();
        let modified_unix = item.modified().to_unix();
        let icon_name = crate::ui::browser::icon_for_name(&file_name);

        files.push(RecentFileItem {
            uri,
            name: file_name,
            parent_path,
            parent_location,
            modified_unix,
            icon_name,
        });
    }

    files.sort_by(|a, b| b.modified_unix.cmp(&a.modified_unix));
    if files.len() > limit {
        files.truncate(limit);
    }
    files
}

pub(crate) struct QuickAccessFolder {
    pub name: String,
    pub path_display: String,
    pub location: Location,
    pub icon: &'static str,
    pub is_pinned: bool,
}

pub(crate) fn load_quick_access_folders() -> Vec<QuickAccessFolder> {
    let mut folders = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();

    let home_dir = super::window::home_directory();
    seen_paths.insert(home_dir.clone());
    folders.push(QuickAccessFolder {
        name: "Personal Folder".into(),
        path_display: "~".into(),
        location: Location::local(home_dir.clone()),
        icon: icons::HOME,
        is_pinned: false,
    });

    let standard_dirs = [
        (glib::UserDirectory::Desktop, "Desktop", icons::FOLDER),
        (glib::UserDirectory::Documents, "Documents", icons::DOCUMENTS),
        (glib::UserDirectory::Downloads, "Downloads", icons::DOWNLOADS),
        (glib::UserDirectory::Music, "Music", icons::MUSIC),
        (glib::UserDirectory::Pictures, "Pictures", icons::PICTURES),
        (glib::UserDirectory::Videos, "Videos", icons::VIDEOS),
    ];

    for (user_dir, name, icon) in standard_dirs {
        if let Some(path) = glib::user_special_dir(user_dir) {
            if path != home_dir && seen_paths.insert(path.clone()) {
                let display = path
                    .strip_prefix(&home_dir)
                    .map(|rel| format!("~/{}", rel.display()))
                    .unwrap_or_else(|_| path.display().to_string());
                folders.push(QuickAccessFolder {
                    name: name.into(),
                    path_display: display,
                    location: Location::local(path),
                    icon,
                    is_pinned: false,
                });
            }
        }
    }

    for (location, name) in super::window::load_pinned_places() {
        if let Some(path) = location.native_path() {
            if seen_paths.insert(path.to_path_buf()) {
                let display = path
                    .strip_prefix(&home_dir)
                    .map(|rel| format!("~/{}", rel.display()))
                    .unwrap_or_else(|_| path.display().to_string());
                folders.push(QuickAccessFolder {
                    name,
                    path_display: display,
                    location,
                    icon: icons::PIN,
                    is_pinned: true,
                });
            }
        }
    }

    folders
}

struct State {
    quick_access_flow: gtk::FlowBox,
    music_container: gtk::Box,
    recent_files_container: gtk::Box,
    browser: BrowserView,
    music_player: Option<super::music::MusicPlayer>,
}

#[derive(Clone)]
pub struct HomeDashboard {
    scroller: gtk::ScrolledWindow,
    state: Rc<State>,
}

impl HomeDashboard {
    pub fn new(
        browser: BrowserView,
        music_player: Option<super::music::MusicPlayer>,
    ) -> Self {
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .build();
        scroller.add_css_class("home-dashboard-scroller");

        let outer_box = gtk::Box::new(gtk::Orientation::Vertical, 28);
        outer_box.add_css_class("home-dashboard-container");
        outer_box.set_hexpand(true);
        outer_box.set_margin_top(28);
        outer_box.set_margin_bottom(36);
        outer_box.set_margin_start(36);
        outer_box.set_margin_end(36);

        // 1. Hero Header
        let header_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let home_icon = assets::primary_icon(icons::HOME, 26);
        let title_label = gtk::Label::new(Some("Home"));
        title_label.add_css_class("home-header-title");
        title_label.set_xalign(0.0);
        title_row.append(&home_icon);
        title_row.append(&title_label);

        let subtitle_label = gtk::Label::new(Some(
            "Quick access to your folders, recent activity, and music",
        ));
        subtitle_label.add_css_class("home-header-subtitle");
        subtitle_label.set_xalign(0.0);

        header_box.append(&title_row);
        header_box.append(&subtitle_label);
        outer_box.append(&header_box);

        // 2. Quick Access Section
        let qa_section = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let qa_header = section_header(icons::PIN, "QUICK ACCESS");
        qa_section.append(&qa_header);

        let quick_access_flow = gtk::FlowBox::new();
        quick_access_flow.add_css_class("home-quick-access-flow");
        quick_access_flow.set_selection_mode(gtk::SelectionMode::None);
        quick_access_flow.set_max_children_per_line(6);
        quick_access_flow.set_min_children_per_line(2);
        quick_access_flow.set_column_spacing(12);
        quick_access_flow.set_row_spacing(12);
        quick_access_flow.set_homogeneous(true);
        qa_section.append(&quick_access_flow);
        outer_box.append(&qa_section);

        // 3. Music Shelf Section
        let music_container = gtk::Box::new(gtk::Orientation::Vertical, 12);
        music_container.add_css_class("home-music-section");
        outer_box.append(&music_container);

        // 4. Recent Files Section
        let recent_section = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let recent_header = section_header(icons::DOCUMENTS, "RECENT FILES");
        recent_section.append(&recent_header);

        let recent_files_container = gtk::Box::new(gtk::Orientation::Vertical, 16);
        recent_files_container.add_css_class("home-recent-files-container");
        recent_section.append(&recent_files_container);
        outer_box.append(&recent_section);

        scroller.set_child(Some(&outer_box));

        let state = Rc::new(State {
            quick_access_flow,
            music_container,
            recent_files_container,
            browser,
            music_player,
        });

        let dashboard = Self { scroller, state };
        dashboard.refresh();

        if let Some(player) = &dashboard.state.music_player {
            let weak_dash = dashboard.downgrade();
            let controller = player.controller();
            let observer: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(dash) = weak_dash.upgrade() {
                    dash.refresh_music();
                }
            });
            controller.observe(&observer);
        }

        dashboard
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.scroller.upcast_ref()
    }

    fn downgrade(&self) -> WeakHomeDashboard {
        WeakHomeDashboard {
            scroller: self.scroller.clone(),
            state: Rc::downgrade(&self.state),
        }
    }

    pub fn refresh(&self) {
        self.refresh_quick_access();
        self.refresh_music();
        self.refresh_recent_files();
    }

    fn refresh_quick_access(&self) {
        while let Some(child) = self.state.quick_access_flow.first_child() {
            self.state.quick_access_flow.remove(&child);
        }

        let folders = load_quick_access_folders();
        for folder in folders {
            let card = gtk::Button::new();
            card.add_css_class("home-folder-card");
            let tooltip = if folder.is_pinned {
                format!("{} (Pinned)", folder.location.display_path())
            } else {
                folder.location.display_path()
            };
            card.set_tooltip_text(Some(&tooltip));

            let card_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let icon = assets::primary_icon(folder.icon, 30);
            icon.set_halign(gtk::Align::Start);
            icon.add_css_class("home-card-icon");

            let name_label = gtk::Label::new(Some(&folder.name));
            name_label.add_css_class("home-card-title");
            name_label.set_xalign(0.0);
            name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);

            let sub_label = gtk::Label::new(Some(&folder.path_display));
            sub_label.add_css_class("home-card-subtitle");
            sub_label.set_xalign(0.0);
            sub_label.set_ellipsize(gtk::pango::EllipsizeMode::End);

            card_box.append(&icon);
            card_box.append(&name_label);
            card_box.append(&sub_label);
            card.set_child(Some(&card_box));

            let browser = self.state.browser.clone();
            let loc = folder.location.clone();
            card.connect_clicked(move |_| {
                browser.navigate_location(loc.clone());
            });

            self.state.quick_access_flow.append(&card);
        }
    }

    fn refresh_music(&self) {
        while let Some(child) = self.state.music_container.first_child() {
            self.state.music_container.remove(&child);
        }

        let Some(player) = &self.state.music_player else {
            self.state.music_container.set_visible(false);
            return;
        };

        let controller = player.controller();
        let track = controller.current_track();
        let is_playing = controller.status() == super::music::controller::PlaybackStatus::Playing;

        self.state.music_container.set_visible(true);
        let header = section_header(icons::MUSIC, "RECENT MUSIC");
        self.state.music_container.append(&header);

        let shelf = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        shelf.add_css_class("home-music-shelf");

        let icon = assets::primary_icon(icons::MUSIC, 36);
        icon.add_css_class("home-music-artwork");
        shelf.append(&icon);

        let meta_box = gtk::Box::new(gtk::Orientation::Vertical, 3);
        meta_box.set_hexpand(true);
        meta_box.set_valign(gtk::Align::Center);

        let title_text = track
            .as_ref()
            .map(|t| t.title())
            .unwrap_or_else(|| "Music Player".into());
        let detail_text = track
            .as_ref()
            .map(|t| format!("{} · {}", t.artist(), t.album()))
            .unwrap_or_else(|| "Play albums, tracks, and audio files".into());

        let title = gtk::Label::new(Some(&title_text));
        title.add_css_class("home-music-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let detail = gtk::Label::new(Some(&detail_text));
        detail.add_css_class("home-music-detail");
        detail.set_xalign(0.0);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);

        meta_box.append(&title);
        meta_box.append(&detail);
        shelf.append(&meta_box);

        let play_btn = gtk::Button::new();
        play_btn.add_css_class("home-music-play-btn");
        let play_icon = if is_playing {
            assets::primary_icon(icons::PAUSE, 18)
        } else {
            assets::primary_icon(icons::PLAY, 18)
        };
        play_btn.set_child(Some(&play_icon));
        play_btn.set_tooltip_text(Some(if is_playing { "Pause" } else { "Play" }));

        let ctrl = controller.clone();
        play_btn.connect_clicked(move |_| {
            ctrl.toggle();
        });
        shelf.append(&play_btn);

        self.state.music_container.append(&shelf);
    }

    fn refresh_recent_files(&self) {
        while let Some(child) = self.state.recent_files_container.first_child() {
            self.state.recent_files_container.remove(&child);
        }

        let files = query_recent_files(25);
        if files.is_empty() {
            let empty_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
            empty_box.add_css_class("home-recent-empty");
            empty_box.set_margin_top(16);
            empty_box.set_margin_bottom(16);

            let empty_label = gtk::Label::new(Some("No recent files yet"));
            empty_label.add_css_class("home-empty-text");
            let empty_sub = gtk::Label::new(Some("Files you open will appear here"));
            empty_sub.add_css_class("home-empty-subtext");

            empty_box.append(&empty_label);
            empty_box.append(&empty_sub);
            self.state.recent_files_container.append(&empty_box);
            return;
        }

        let Ok(now) = glib::DateTime::now_local() else {
            return;
        };

        let buckets = [
            DateBucket::Today,
            DateBucket::Yesterday,
            DateBucket::EarlierThisWeek,
            DateBucket::Older,
        ];

        for bucket in buckets {
            let bucket_files: Vec<_> = files
                .iter()
                .filter(|f| DateBucket::from_timestamp(f.modified_unix, &now) == bucket)
                .collect();

            if bucket_files.is_empty() {
                continue;
            }

            let bucket_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let bucket_label = gtk::Label::new(Some(bucket.label()));
            bucket_label.add_css_class("home-bucket-label");
            bucket_label.set_xalign(0.0);
            bucket_box.append(&bucket_label);

            let list_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
            list_box.add_css_class("home-recent-list");

            for file in bucket_files {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                row.add_css_class("home-recent-row");

                let icon = assets::primary_icon(file.icon_name, 20);
                icon.add_css_class("home-file-icon");
                row.append(&icon);

                let text_col = gtk::Box::new(gtk::Orientation::Vertical, 2);
                text_col.set_hexpand(true);

                let name = gtk::Label::new(Some(&file.name));
                name.add_css_class("home-file-name");
                name.set_xalign(0.0);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);

                let folder = gtk::Label::new(Some(&file.parent_path));
                folder.add_css_class("home-file-folder");
                folder.set_xalign(0.0);
                folder.set_ellipsize(gtk::pango::EllipsizeMode::End);

                text_col.append(&name);
                text_col.append(&folder);
                row.append(&text_col);

                let time_str = format_timestamp(file.modified_unix);
                let time_label = gtk::Label::new(Some(&time_str));
                time_label.add_css_class("home-file-time");
                row.append(&time_label);

                let open_folder_btn = gtk::Button::new();
                open_folder_btn.add_css_class("home-row-action-btn");
                open_folder_btn.set_tooltip_text(Some("Open containing folder"));
                let folder_icon = assets::primary_icon(icons::FOLDER, 16);
                open_folder_btn.set_child(Some(&folder_icon));

                let browser = self.state.browser.clone();
                let parent_loc = file.parent_location.clone();
                open_folder_btn.connect_clicked(move |_| {
                    browser.navigate_location(parent_loc.clone());
                });
                row.append(&open_folder_btn);

                let click = gtk::GestureClick::new();
                let uri = file.uri.clone();
                click.connect_pressed(move |_, _, _, _| {
                    let _ = gio::AppInfo::launch_default_for_uri(
                        &uri,
                        None::<&gio::AppLaunchContext>,
                    );
                });
                row.add_controller(click);

                list_box.append(&row);
            }

            bucket_box.append(&list_box);
            self.state.recent_files_container.append(&bucket_box);
        }
    }
}

struct WeakHomeDashboard {
    scroller: gtk::ScrolledWindow,
    state: std::rc::Weak<State>,
}

impl WeakHomeDashboard {
    fn upgrade(&self) -> Option<HomeDashboard> {
        self.state
            .upgrade()
            .map(|state| HomeDashboard {
                scroller: self.scroller.clone(),
                state,
            })
    }
}

fn section_header(icon_name: &str, title: &str) -> gtk::Box {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("home-section-header");
    let icon = assets::primary_icon(icon_name, 16);
    let label = gtk::Label::new(Some(title));
    label.add_css_class("home-section-title");
    label.set_xalign(0.0);
    header.append(&icon);
    header.append(&label);
    header
}

#[cfg(test)]
mod tests;
