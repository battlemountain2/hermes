// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(1);

struct TempZip(PathBuf);

impl Drop for TempZip {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn create_sample_zip() -> TempZip {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("hermes-zip-test-{}-{}.zip", std::process::id(), id));
    let file = std::fs::File::create(&path).expect("create temp zip");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();

    zip.start_file("Root Document.txt", options).unwrap();
    zip.write_all(b"Hello from root").unwrap();

    zip.start_file("Subdir/Inner File.txt", options).unwrap();
    zip.write_all(b"Hello from inside").unwrap();

    zip.finish().unwrap();
    TempZip(path)
}

#[test]
fn test_archive_index_parsing_and_subpaths() {
    let zip_file = create_sample_zip();
    let index = parse_archive_contents(&zip_file.0).expect("parse zip");

    // Root should contain "Subdir" (dir) and "Root Document.txt" (file)
    let root_children = index.children.get("").expect("root children exist");
    assert_eq!(root_children.len(), 2);

    let subdir_node = root_children.iter().find(|n| n.name == "Subdir").unwrap();
    assert!(subdir_node.is_dir);
    assert_eq!(subdir_node.relative_path, "Subdir");

    let doc_node = root_children
        .iter()
        .find(|n| n.name == "Root Document.txt")
        .unwrap();
    assert!(!doc_node.is_dir);
    assert_eq!(doc_node.size, 15);

    // Subdir should contain "Inner File.txt"
    let inner_children = index.children.get("Subdir").expect("subdir children exist");
    assert_eq!(inner_children.len(), 1);
    assert_eq!(inner_children[0].name, "Inner File.txt");
    assert_eq!(inner_children[0].relative_path, "Subdir/Inner File.txt");
    assert!(!inner_children[0].is_dir);
}

#[test]
fn test_archive_source_validation() {
    let zip_file = create_sample_zip();
    let source = ArchiveFileSource::new();

    let valid_loc = Location::uri(format!("archive://{}/", zip_file.0.display()));
    assert!(source.validate_location(&valid_loc).is_ok());

    let invalid_loc = Location::uri("archive:///nonexistent/path/archive.zip/");
    assert!(source.validate_location(&invalid_loc).is_err());
}
