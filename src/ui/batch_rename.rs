use gtk::{glib, prelude::*, subclass::prelude::*};
use std::rc::Rc;
use crate::model::FileEntry;

glib::wrapper! {
    pub struct BatchRenameDialog(ObjectSubclass<imp::BatchRenameDialog>)
        @extends gtk::Widget, gtk::Window,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct BatchRenameDialog {
        pub entries: RefCell<Vec<FileEntry>>,
        pub callback: RefCell<Option<Box<dyn Fn(Vec<(FileEntry, String)>)>>>,
        pub list_box: gtk::ListBox,
        pub find_entry: gtk::Entry,
        pub replace_entry: gtk::Entry,
        pub apply_button: gtk::Button,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BatchRenameDialog {
        const NAME: &'static str = "HermesBatchRenameDialog";
        type Type = super::BatchRenameDialog;
        type ParentType = gtk::Window;
    }

    impl ObjectImpl for BatchRenameDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            
            obj.set_title(Some("Batch Rename"));
            obj.set_default_size(400, 300);

            let main_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
            main_box.set_margin_top(12);
            main_box.set_margin_bottom(12);
            main_box.set_margin_start(12);
            main_box.set_margin_end(12);
            obj.set_child(Some(&main_box));

            let grid = gtk::Grid::new();
            grid.set_column_spacing(6);
            grid.set_row_spacing(6);

            let find_label = gtk::Label::new(Some("Find:"));
            grid.attach(&find_label, 0, 0, 1, 1);
            grid.attach(&self.find_entry, 1, 0, 1, 1);
            self.find_entry.set_hexpand(true);

            let replace_label = gtk::Label::new(Some("Replace with:"));
            grid.attach(&replace_label, 0, 1, 1, 1);
            grid.attach(&self.replace_entry, 1, 1, 1, 1);
            self.replace_entry.set_hexpand(true);

            main_box.append(&grid);

            let scrolled_window = gtk::ScrolledWindow::new();
            scrolled_window.set_vexpand(true);
            scrolled_window.set_min_content_height(200);
            scrolled_window.set_child(Some(&self.list_box));
            main_box.append(&scrolled_window);

            let action_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            action_box.set_halign(gtk::Align::End);
            
            let cancel_button = gtk::Button::with_label("Cancel");
            
            self.apply_button.set_label("Apply");
            self.apply_button.add_css_class("suggested-action");
            
            action_box.append(&cancel_button);
            action_box.append(&self.apply_button);
            main_box.append(&action_box);

            let find_clone = self.find_entry.clone();
            let replace_clone = self.replace_entry.clone();
            let list_box = self.list_box.clone();
            let entries = self.entries.clone();
            
            let update_preview = move || {
                let find_text = find_clone.text().to_string();
                let replace_text = replace_clone.text().to_string();
                
                while let Some(child) = list_box.first_child() {
                    list_box.remove(&child);
                }

                for entry in entries.borrow().iter() {
                    let old_name = entry.display_name.clone();
                    let new_name = if find_text.is_empty() {
                        old_name.clone()
                    } else {
                        old_name.replace(&find_text, &replace_text)
                    };

                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                    let old_label = gtk::Label::new(Some(&old_name));
                    old_label.set_xalign(0.0);
                    old_label.set_hexpand(true);
                    
                    let arrow = gtk::Image::from_icon_name("go-next-symbolic");
                    
                    let new_label = gtk::Label::new(Some(&new_name));
                    new_label.set_xalign(0.0);
                    new_label.set_hexpand(true);

                    row.append(&old_label);
                    row.append(&arrow);
                    row.append(&new_label);

                    list_box.append(&row);
                }
            };

            let update_preview_rc = Rc::new(update_preview);
            
            // Initial render
            update_preview_rc();
            
            let update1 = update_preview_rc.clone();
            self.find_entry.connect_changed(move |_| update1());
            
            let update2 = update_preview_rc.clone();
            self.replace_entry.connect_changed(move |_| update2());

            let weak_obj = obj.downgrade();
            let find_clone2 = self.find_entry.clone();
            let replace_clone2 = self.replace_entry.clone();
            let entries2 = self.entries.clone();
            
            self.apply_button.connect_clicked(move |_| {
                if let Some(obj) = weak_obj.upgrade() {
                    let imp = obj.imp();
                    if let Some(cb) = imp.callback.borrow_mut().take() {
                        let find_text = find_clone2.text().to_string();
                        let replace_text = replace_clone2.text().to_string();
                        
                        let renames = entries2.borrow().iter().map(|entry| {
                            let old_name = entry.display_name.clone();
                            let new_name = if find_text.is_empty() {
                                old_name.clone()
                            } else {
                                old_name.replace(&find_text, &replace_text)
                            };
                            (entry.clone(), new_name)
                        }).filter(|(e, n)| &e.display_name != n).collect();
                        
                        cb(renames);
                    }
                    obj.close();
                }
            });

            let weak_obj_cancel = obj.downgrade();
            cancel_button.connect_clicked(move |_| {
                if let Some(obj) = weak_obj_cancel.upgrade() {
                    obj.close();
                }
            });
        }
    }

    impl WidgetImpl for BatchRenameDialog {}
    impl WindowImpl for BatchRenameDialog {}
}

impl BatchRenameDialog {
    pub fn new(entries: Vec<FileEntry>, callback: impl Fn(Vec<(FileEntry, String)>) + 'static) -> Self {
        let obj: Self = glib::Object::builder().build();
        let imp = obj.imp();
        *imp.entries.borrow_mut() = entries;
        *imp.callback.borrow_mut() = Some(Box::new(callback));
        obj
    }
}
