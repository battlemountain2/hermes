use super::*;

#[test]
fn basic_copy() {
    let name = collision_safe_name("hello.txt", &|_| false);
    assert_eq!(name, "hello (copy).txt");
}

#[test]
fn second_copy() {
    let name = collision_safe_name("hello.txt", &|n| n == "hello (copy).txt");
    assert_eq!(name, "hello (copy 2).txt");
}

#[test]
fn third_copy() {
    let name = collision_safe_name("hello.txt", &|n| {
        n == "hello (copy).txt" || n == "hello (copy 2).txt"
    });
    assert_eq!(name, "hello (copy 3).txt");
}

#[test]
fn no_extension() {
    let name = collision_safe_name("Makefile", &|_| false);
    assert_eq!(name, "Makefile (copy)");
}

#[test]
fn dotfile() {
    let name = collision_safe_name(".gitignore", &|_| false);
    assert_eq!(name, ".gitignore (copy)");
}

#[test]
fn directory_name() {
    let name = collision_safe_name("Documents", &|_| false);
    assert_eq!(name, "Documents (copy)");
}

#[test]
fn double_extension() {
    let name = collision_safe_name("archive.tar.gz", &|_| false);
    assert_eq!(name, "archive.tar (copy).gz");
}

#[test]
fn unicode_name() {
    let name = collision_safe_name("文档.pdf", &|_| false);
    assert_eq!(name, "文档 (copy).pdf");
}

#[test]
fn many_existing_copies() {
    let name = collision_safe_name("file.txt", &|n| {
        n == "file (copy).txt"
            || n == "file (copy 2).txt"
            || n == "file (copy 3).txt"
            || n == "file (copy 4).txt"
            || n == "file (copy 5).txt"
    });
    assert_eq!(name, "file (copy 6).txt");
}

#[test]
fn split_simple() {
    assert_eq!(split_stem_extension("photo.jpg"), ("photo", "jpg"));
}

#[test]
fn split_no_extension() {
    assert_eq!(split_stem_extension("Makefile"), ("Makefile", ""));
}

#[test]
fn split_dotfile() {
    assert_eq!(split_stem_extension(".gitignore"), (".gitignore", ""));
}

#[test]
fn split_dotfile_with_ext() {
    assert_eq!(split_stem_extension(".bashrc.bak"), (".bashrc", "bak"));
}

#[test]
fn split_double_ext() {
    assert_eq!(split_stem_extension("archive.tar.gz"), ("archive.tar", "gz"));
}
