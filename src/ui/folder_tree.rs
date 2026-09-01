use gtk::{gio, glib, prelude::*};
use glib::subclass::prelude::*;
use std::rc::Rc;
use crate::ui::browser::BrowserView;
use crate::model::Location;

mod imp {
    use super::*;
    use glib::subclass::prelude::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct TreeNode {
        pub location: RefCell<Option<Location>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TreeNode {
        const NAME: &'static str = "StrataTreeNode";
        type Type = super::TreeNode;
    }

    impl ObjectImpl for TreeNode {}
}

glib::wrapper! {
    pub struct TreeNode(ObjectSubclass<imp::TreeNode>);
}

impl TreeNode {
    pub fn new(location: Location) -> Self {
        let obj: Self = glib::Object::builder().build();
        obj.imp().location.replace(Some(location));
        obj
    }
    
    pub fn location(&self) -> Location {
        self.imp().location.borrow().as_ref().unwrap().clone()
    }
}

pub fn build_folder_tree(browser: BrowserView) -> gtk::Widget {
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    scrolled.add_css_class("folder-tree-panel");

    let root_store = gio::ListStore::new::<TreeNode>();
    
    // Add home directory
    let home = glib::home_dir();
    let loc = Location::local(home);
    root_store.append(&TreeNode::new(loc));
    
    // Add root directory
    root_store.append(&TreeNode::new(Location::local("/")));

    let tree_model = gtk::TreeListModel::new(
        root_store.clone(),
        false,
        false,
        |item| {
            let node = item.downcast_ref::<TreeNode>()?;
            let loc = node.location();
            
            // For now, return a new ListStore populated synchronously for children
            let store = gio::ListStore::new::<TreeNode>();
            if let Some(path) = loc.native_path() {
                if let Ok(dir) = std::fs::read_dir(path) {
                    let mut entries: Vec<_> = dir.filter_map(Result::ok).collect();
                    entries.sort_by_key(|e| e.file_name());
                    for entry in entries {
                        if let Ok(ft) = entry.file_type() {
                            if ft.is_dir() {
                                let child_loc = Location::local(entry.path());
                                store.append(&TreeNode::new(child_loc));
                            }
                        }
                    }
                }
            }
            Some(store.upcast())
        },
    );

    let selection = gtk::SingleSelection::new(Some(tree_model));
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return; };
        
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let expander = gtk::TreeExpander::new();
        let icon = gtk::Image::new();
        let label = gtk::Label::new(None);
        
        row.append(&icon);
        row.append(&label);
        expander.set_child(Some(&row));
        item.set_child(Some(&expander));
    });

    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return; };
        let Some(tree_row) = item.item().and_downcast::<gtk::TreeListRow>() else { return; };
        let Some(node) = tree_row.item().and_downcast::<TreeNode>() else { return; };
        let Some(expander) = item.child().and_downcast::<gtk::TreeExpander>() else { return; };
        
        expander.set_list_row(Some(&tree_row));
        
        let row = expander.child().unwrap().downcast::<gtk::Box>().unwrap();
        let icon = row.first_child().unwrap().downcast::<gtk::Image>().unwrap();
        let label = icon.next_sibling().unwrap().downcast::<gtk::Label>().unwrap();
        
        let loc = node.location();
        let name = loc.display_name();
        
        crate::assets::set_primary_icon(&icon, crate::assets::icons::FOLDER);
        label.set_label(&name);
    });

    let list_view = gtk::ListView::new(Some(selection.clone()), Some(factory));
    list_view.add_css_class("folder-tree-list");
    
    let browser_for_tree = browser.clone();
    list_view.connect_activate(move |list, position| {
        let Some(model) = list.model() else { return };
        let Some(tree_row) = model.item(position).and_downcast::<gtk::TreeListRow>() else { return };
        let Some(node) = tree_row.item().and_downcast::<TreeNode>() else { return };
        
        if let Some(path) = node.location().native_path() {
            browser_for_tree.navigate(path);
        }
    });

    scrolled.set_child(Some(&list_view));
    scrolled.upcast()
}
