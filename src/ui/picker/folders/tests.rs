use super::*;
use crate::sandbox::browser::format_tests::FixtureDirectory;

#[test]
fn creating_a_folder_preserves_existing_items_and_rejects_traversal() {
    let fixture = FixtureDirectory::new();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let parent = gio::File::for_path(&fixture.0);
                let folder = create_folder(parent.clone(), "New résumé folder")
                    .await
                    .unwrap();
                assert_eq!(folder.path().unwrap(), fixture.0.join("New résumé folder"));
                std::fs::write(folder.path().unwrap().join("keep"), b"original").unwrap();
                assert!(
                    create_folder(parent.clone(), "New résumé folder")
                        .await
                        .unwrap_err()
                        .contains("already exists")
                );
                assert_eq!(
                    std::fs::read(folder.path().unwrap().join("keep")).unwrap(),
                    b"original"
                );
                for name in ["", ".", "..", "../outside", "nested/child", "/absolute"] {
                    assert!(create_folder(parent.clone(), name).await.is_err());
                }
                assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 1);
            })
        })
        .unwrap();
}
