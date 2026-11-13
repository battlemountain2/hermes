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
                    .expect("expected test value from create_folder(parent.clone(), \"New résumé folder\") .await");
                assert_eq!(folder.path().expect("expected test value from folder.path()"), fixture.0.join("New résumé folder"));
                std::fs::write(folder.path().expect("expected test value from folder.path()").join("keep"), b"original").expect("write generated fixture");
                assert!(
                    create_folder(parent.clone(), "New résumé folder")
                        .await
                        .expect_err("invalid or cancelled fixture must fail")
                        .contains("already exists")
                );
                assert_eq!(
                    std::fs::read(folder.path().expect("expected test value from folder.path()").join("keep")).expect("read generated fixture"),
                    b"original"
                );
                for name in ["", ".", "..", "../outside", "nested/child", "/absolute"] {
                    assert!(create_folder(parent.clone(), name).await.is_err());
                }
                assert_eq!(std::fs::read_dir(&fixture.0).expect("expected test value from std::fs::read_dir(&fixture.0)").count(), 1);
            })
        })
        .expect("write generated fixture");
}
