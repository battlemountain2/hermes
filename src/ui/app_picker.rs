// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, collections::HashSet, path::Path, rc::Rc};

use gtk::{gio, prelude::*};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Section {
    Default,
    Recommended,
    Other,
}

struct ChoiceRow {
    button: gtk::ToggleButton,
    search_text: String,
    section: Section,
}

struct SectionView {
    heading: gtk::Label,
}

#[expect(
    deprecated,
    reason = "GTK 4.10 dialog replacements do not provide a custom application picker"
)]
pub(super) fn present(
    parent: &gtk::Widget,
    file: &gio::File,
    content_type: &str,
    report_error: Rc<dyn Fn(String)>,
) {
    let Some(window) = parent.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let display_name = file
        .path()
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .or_else(|| {
            file.basename()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "selected file".to_owned());
    let type_description = gio::content_type_get_description(content_type);

    let dialog = gtk::Dialog::builder()
        .transient_for(&window)
        .modal(true)
        .title("Open With")
        .default_width(640)
        .default_height(620)
        .build();
    dialog.add_css_class("hermes-app-picker");
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    let open = dialog.add_button("Open", gtk::ResponseType::Accept);
    open.add_css_class("suggested-action");
    open.set_sensitive(false);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("app-picker-content");
    let title = gtk::Label::new(Some(&format!("Open “{display_name}” with")));
    title.add_css_class("app-picker-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let subtitle = gtk::Label::new(Some(&type_description));
    subtitle.add_css_class("app-picker-subtitle");
    subtitle.set_xalign(0.0);
    content.append(&title);
    content.append(&subtitle);

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search installed applications")
        .build();
    search.add_css_class("app-picker-search");
    content.append(&search);

    let applications = gtk::Box::new(gtk::Orientation::Vertical, 4);
    applications.add_css_class("app-picker-list");
    let default_section = append_section_heading(&applications, "DEFAULT APPLICATION");
    let recommended_section = append_section_heading(&applications, "RECOMMENDED");
    let other_section = append_section_heading(&applications, "ALL OTHER APPLICATIONS");

    let default = gio::AppInfo::default_for_type(content_type, false).filter(can_open_file);
    let recommended = gio::AppInfo::recommended_for_type(content_type);
    let all = gio::AppInfo::all();
    let rows = Rc::new(RefCell::new(Vec::<ChoiceRow>::new()));
    let selected = Rc::new(RefCell::new(None::<gio::AppInfo>));
    let first_button = Rc::new(RefCell::new(None::<gtk::ToggleButton>));
    let mut seen = HashSet::new();

    if let Some(app) = default {
        append_application(
            &applications,
            &rows,
            &selected,
            &first_button,
            &open,
            &dialog,
            &mut seen,
            app,
            Section::Default,
            true,
        );
    }
    for app in recommended.into_iter().filter(show_application) {
        append_application(
            &applications,
            &rows,
            &selected,
            &first_button,
            &open,
            &dialog,
            &mut seen,
            app,
            Section::Recommended,
            false,
        );
    }
    for app in all.into_iter().filter(show_application) {
        append_application(
            &applications,
            &rows,
            &selected,
            &first_button,
            &open,
            &dialog,
            &mut seen,
            app,
            Section::Other,
            false,
        );
    }

    update_section_visibility(
        &rows.borrow(),
        &default_section,
        &recommended_section,
        &other_section,
    );
    let empty = gtk::Label::new(Some("No installed applications found"));
    empty.add_css_class("app-picker-empty");
    empty.set_visible(rows.borrow().is_empty());
    applications.append(&empty);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(320)
        .vexpand(true)
        .child(&applications)
        .build();
    scroller.add_css_class("app-picker-scroll");
    content.append(&scroller);

    let remember = gtk::CheckButton::with_label(&format!(
        "Always use this application for {type_description} files"
    ));
    remember.add_css_class("app-picker-remember");
    remember.set_sensitive(!rows.borrow().is_empty());
    content.append(&remember);
    dialog.content_area().append(&content);

    if let Some(first) = first_button.borrow().as_ref() {
        first.set_active(true);
    }
    open.set_sensitive(selected.borrow().is_some());
    let rows_for_search = rows.clone();
    let selected_for_search = selected.clone();
    let open_for_search = open.clone();
    let empty_for_search = empty.clone();
    let default_for_search = default_section.heading.clone();
    let recommended_for_search = recommended_section.heading.clone();
    let other_for_search = other_section.heading.clone();
    search.connect_search_changed(move |search| {
        let query = search.text().trim().to_lowercase();
        for row in rows_for_search.borrow().iter() {
            row.button
                .set_visible(query.is_empty() || row.search_text.contains(&query));
        }
        let has_visible_selection = rows_for_search
            .borrow()
            .iter()
            .any(|row| row.button.is_visible() && row.button.is_active());
        if !has_visible_selection {
            if let Some(row) = rows_for_search
                .borrow()
                .iter()
                .find(|row| row.button.is_visible())
            {
                row.button.set_active(true);
                open_for_search.set_sensitive(true);
            } else {
                selected_for_search.replace(None);
                open_for_search.set_sensitive(false);
            }
        }
        empty_for_search.set_text("No matching applications");
        empty_for_search.set_visible(
            !rows_for_search
                .borrow()
                .iter()
                .any(|row| row.button.is_visible()),
        );
        update_section_visibility(
            &rows_for_search.borrow(),
            &SectionView {
                heading: default_for_search.clone(),
            },
            &SectionView {
                heading: recommended_for_search.clone(),
            },
            &SectionView {
                heading: other_for_search.clone(),
            },
        );
    });

    let selected_for_response = selected.clone();
    let file_for_response = file.clone();
    let content_type = content_type.to_owned();
    dialog.connect_response(move |dialog, response| {
        if response != gtk::ResponseType::Accept {
            dialog.close();
            return;
        }
        let Some(app) = selected_for_response.borrow().clone() else {
            return;
        };
        let mut failures = Vec::new();
        if remember.is_active()
            && let Err(error) = app.set_as_default_for_type(&content_type)
        {
            failures.push(format!("Could not save the default application: {error}"));
        }
        if let Err(error) = app.launch(
            std::slice::from_ref(&file_for_response),
            None::<&gio::AppLaunchContext>,
        ) {
            failures.push(format!("Could not open the file: {error}"));
        }
        dialog.close();
        if !failures.is_empty() {
            report_error(failures.join("\n"));
        }
    });
    dialog.present();
}

fn append_section_heading(container: &gtk::Box, text: &str) -> SectionView {
    let heading = gtk::Label::new(Some(text));
    heading.add_css_class("app-picker-section-title");
    heading.set_xalign(0.0);
    container.append(&heading);
    SectionView { heading }
}

#[allow(
    clippy::too_many_arguments,
    reason = "keeps application-row assembly centralized"
)]
#[expect(
    deprecated,
    reason = "GTK 4.10 dialog replacements do not provide a custom application picker"
)]
fn append_application(
    container: &gtk::Box,
    rows: &Rc<RefCell<Vec<ChoiceRow>>>,
    selected: &Rc<RefCell<Option<gio::AppInfo>>>,
    first_button: &Rc<RefCell<Option<gtk::ToggleButton>>>,
    open: &gtk::Widget,
    dialog: &gtk::Dialog,
    seen: &mut HashSet<String>,
    app: gio::AppInfo,
    section: Section,
    is_default: bool,
) {
    let key = application_key(&app);
    if !seen.insert(key) {
        return;
    }
    let button = gtk::ToggleButton::new();
    button.add_css_class("app-picker-row");
    button.set_has_frame(false);
    if let Some(first) = first_button.borrow().as_ref() {
        button.set_group(Some(first));
    } else {
        first_button.replace(Some(button.clone()));
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = app
        .icon()
        .map(|icon| gtk::Image::from_gicon(&icon))
        .unwrap_or_else(|| gtk::Image::from_icon_name(crate::assets::icons::EXTERNAL_LINK));
    icon.set_pixel_size(32);
    icon.add_css_class("app-picker-icon");
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 1);
    labels.set_hexpand(true);
    let name = gtk::Label::new(Some(&app.display_name()));
    name.add_css_class("app-picker-app-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let description = app
        .description()
        .map(|value| value.to_string())
        .filter(|value| !value.eq_ignore_ascii_case(&app.display_name()))
        .unwrap_or_else(|| app.executable().to_string_lossy().into_owned());
    let detail = gtk::Label::new(Some(&description));
    detail.add_css_class("app-picker-app-description");
    detail.set_xalign(0.0);
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    labels.append(&name);
    labels.append(&detail);
    row.append(&icon);
    row.append(&labels);
    if is_default {
        let badge = gtk::Label::new(Some("Default"));
        badge.add_css_class("app-picker-default-badge");
        row.append(&badge);
    }
    button.set_child(Some(&row));
    let selected_for_toggle = selected.clone();
    let app_for_toggle = app.clone();
    let open_for_toggle = open.clone();
    button.connect_toggled(move |button| {
        if button.is_active() {
            selected_for_toggle.replace(Some(app_for_toggle.clone()));
            open_for_toggle.set_sensitive(true);
        }
    });
    let activate = gtk::GestureClick::new();
    let dialog_for_activate = dialog.clone();
    activate.connect_released(move |_, presses, _, _| {
        if presses == 2 {
            dialog_for_activate.response(gtk::ResponseType::Accept);
        }
    });
    button.add_controller(activate);
    let search_text = format!(
        "{} {} {}",
        app.display_name(),
        description,
        app.executable().display()
    )
    .to_lowercase();
    rows.borrow_mut().push(ChoiceRow {
        button: button.clone(),
        search_text,
        section,
    });
    container.append(&button);
}

fn update_section_visibility(
    rows: &[ChoiceRow],
    default: &SectionView,
    recommended: &SectionView,
    other: &SectionView,
) {
    for (section, heading) in [
        (Section::Default, &default.heading),
        (Section::Recommended, &recommended.heading),
        (Section::Other, &other.heading),
    ] {
        heading.set_visible(
            rows.iter()
                .any(|row| row.section == section && row.button.is_visible()),
        );
    }
}

fn show_application(app: &gio::AppInfo) -> bool {
    app.should_show() && can_open_file(app)
}

fn can_open_file(app: &gio::AppInfo) -> bool {
    app.supports_files() || app.supports_uris()
}

fn application_key(app: &gio::AppInfo) -> String {
    app.id()
        .map(|id| id.to_string())
        .unwrap_or_else(|| format!("{}\0{}", app.display_name(), app.executable().display()))
}

#[cfg(test)]
mod tests;
