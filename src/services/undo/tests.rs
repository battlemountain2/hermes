use super::*;

#[test]
fn push_and_pop() {
    let mut history = UndoHistory::new();
    assert!(!history.can_undo());
    assert_eq!(history.len(), 0);

    history.push(UndoEntry::Create {
        location: Location::local("/tmp/test"),
    });
    assert!(history.can_undo());
    assert_eq!(history.len(), 1);

    let entry = history.pop().unwrap();
    assert!(matches!(entry, UndoEntry::Create { .. }));
    assert!(!history.can_undo());
}

#[test]
fn lifo_order() {
    let mut history = UndoHistory::new();
    history.push(UndoEntry::Create {
        location: Location::local("/tmp/first"),
    });
    history.push(UndoEntry::Create {
        location: Location::local("/tmp/second"),
    });

    let entry = history.pop().unwrap();
    if let UndoEntry::Create { location } = entry {
        assert_eq!(location.display_path(), "/tmp/second");
    } else {
        panic!("Expected Create");
    }

    let entry = history.pop().unwrap();
    if let UndoEntry::Create { location } = entry {
        assert_eq!(location.display_path(), "/tmp/first");
    } else {
        panic!("Expected Create");
    }
}

#[test]
fn capacity_bounded() {
    let mut history = UndoHistory::new();
    for i in 0..25 {
        history.push(UndoEntry::Create {
            location: Location::local(format!("/tmp/file{i}")),
        });
    }
    assert_eq!(history.len(), 20);

    // Oldest entries should have been evicted; first remaining is file5
    let entry = history.pop().unwrap();
    if let UndoEntry::Create { location } = entry {
        assert_eq!(location.display_path(), "/tmp/file24");
    } else {
        panic!("Expected Create");
    }
}

#[test]
fn describe_last() {
    let mut history = UndoHistory::new();
    assert!(history.describe_last().is_none());

    history.push(UndoEntry::Rename {
        location: Location::local("/tmp/old.txt"),
        old_name: "old.txt".to_owned(),
        new_name: "new.txt".to_owned(),
    });
    assert_eq!(
        history.describe_last().unwrap(),
        "Undo rename of old.txt"
    );
}

#[test]
fn clear_empties_history() {
    let mut history = UndoHistory::new();
    history.push(UndoEntry::Create {
        location: Location::local("/tmp/test"),
    });
    history.push(UndoEntry::Create {
        location: Location::local("/tmp/test2"),
    });
    assert_eq!(history.len(), 2);

    history.clear();
    assert_eq!(history.len(), 0);
    assert!(!history.can_undo());
}

#[test]
fn trash_description_single() {
    use crate::model::{EntryKind, MetadataValue};
    let entry = UndoEntry::Trash {
        entries: vec![crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner {
            location: Location::local("/tmp/doc.txt"),
            native_name: "doc.txt".into(),
            display_name: "doc.txt".to_owned(),
            kind: EntryKind::File,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
        }))],
    };
    assert_eq!(entry.description(), "Undo trash of doc.txt");
}

#[test]
fn trash_description_multiple() {
    use crate::model::{EntryKind, MetadataValue};
    let entry = UndoEntry::Trash {
        entries: vec![
            crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner {
                location: Location::local("/tmp/a.txt"),
                native_name: "a.txt".into(),
                display_name: "a.txt".to_owned(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
            })),
            crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner {
                location: Location::local("/tmp/b.txt"),
                native_name: "b.txt".into(),
                display_name: "b.txt".to_owned(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
            })),
        ],
    };
    assert_eq!(entry.description(), "Undo trash of 2 items");
}
