use super::*;

#[test]
fn virtual_searches_are_not_recent_physical_folders_or_drop_destinations() {
    assert!(!is_folder_history_location(&Location::uri(
        "search://modified:today"
    )));
    assert!(!is_folder_history_location(&Location::uri(
        "search://?scope=folder&root=/photos&q=type:image"
    )));
    assert!(is_folder_history_location(&Location::local("/photos")));
    assert!(is_folder_history_location(&Location::uri(
        "smb://server/photos"
    )));
    assert!(!is_folder_history_location(&Location::uri(
        "home:///"
    )));
}
