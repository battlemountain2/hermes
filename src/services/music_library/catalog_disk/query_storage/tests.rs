use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Fixture {
    parent: PathBuf,
    root: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::temp_dir().join(format!(
            "hermes-query-files-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&parent)
            .expect("private fixture");
        let root = parent.join("Root");
        fs::create_dir(&root).expect("root fixture");
        Self {
            root,
            path: parent.join("cache/query.sqlite"),
            parent,
        }
    }
    fn open(&self) -> Staged {
        Staged::open(&self.path, &self.root, &Cancellation::default())
            .expect("open private query stage")
    }
    fn save(&self, value: i64) -> Database {
        let staged = self.open();
        staged
            .connection()
            .execute_batch("CREATE TABLE value(number INTEGER) STRICT;")
            .expect("schema");
        staged
            .connection()
            .execute("INSERT INTO value VALUES(?1)", [value])
            .expect("row");
        staged
            .publish(&Cancellation::default())
            .expect("publish cache")
    }
    fn stages(&self) -> usize {
        fs::read_dir(self.path.parent().expect("cache parent"))
            .expect("cache directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".snapshot-")
            })
            .count()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.parent);
    }
}
fn value(database: &Database) -> i64 {
    database
        .connection()
        .query_row("SELECT number FROM value", [], |row| row.get(0))
        .expect("read pinned revision")
}

#[test]
fn publication_keeps_private_readers_isolated_and_releases_the_cache_lock() {
    let fixture = Fixture::new();
    let first = fixture.save(1);
    assert_eq!(fixture.stages(), 1);
    let orphan = fixture
        .path
        .parent()
        .expect("cache parent")
        .join(".snapshot-v2-123-999");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&orphan)
        .expect("generated abandoned stage");
    let next = fixture.open();
    assert!(!orphan.exists());
    assert!(next.reused());
    next.connection()
        .execute("UPDATE value SET number=2", [])
        .expect("update private stage");
    assert_eq!(value(&first), 1);
    let second = next
        .publish(&Cancellation::default())
        .expect("publish second revision");
    assert_eq!((value(&first), value(&second)), (1, 2));
    assert_eq!(fixture.stages(), 2);
    drop(first);
    assert_eq!(fixture.stages(), 1);
    drop(second);
    assert_eq!(fixture.stages(), 0);
}

#[test]
fn dropping_or_cancelling_a_build_preserves_the_public_database() {
    let fixture = Fixture::new();
    drop(fixture.save(7));
    let original = fs::read(&fixture.path).expect("public bytes");
    let staged = fixture.open();
    staged
        .connection()
        .execute("UPDATE value SET number=8", [])
        .expect("private update");
    drop(staged);
    assert_eq!(fs::read(&fixture.path).expect("public bytes"), original);
    let staged = fixture.open();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(staged.publish(&cancellation).is_err());
    assert_eq!(fs::read(&fixture.path).expect("public bytes"), original);
    assert_eq!(fixture.stages(), 0);
}

#[test]
fn malformed_private_cache_can_rebuild_without_early_public_mutation() {
    let fixture = Fixture::new();
    drop(fixture.save(1));
    let malformed = b"generated invalid SQLite database";
    fs::write(&fixture.path, malformed).expect("corrupt disposable fixture");
    let staged = fixture.open();
    assert!(!staged.reused());
    assert_eq!(
        fs::read(&fixture.path).expect("preserved invalid cache"),
        malformed
    );
    staged
        .connection()
        .execute_batch("CREATE TABLE value(number INTEGER) STRICT; INSERT INTO value VALUES(9);")
        .expect("fresh schema");
    let database = staged
        .publish(&Cancellation::default())
        .expect("replace with complete database");
    assert_eq!(value(&database), 9);
}

#[test]
fn reset_only_discards_the_private_build() {
    let fixture = Fixture::new();
    drop(fixture.save(1));
    let original = fs::read(&fixture.path).expect("public bytes");
    let mut staged = fixture.open();
    let root_key = staged.root_key();
    assert_eq!(root_key.len(), 16);
    staged.reset().expect("reset stage");
    assert!(!staged.reused());
    let count: i64 = staged
        .connection()
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
        .expect("empty schema");
    assert_eq!(count, 0);
    assert_eq!(staged.root_key(), root_key);
    drop(staged);
    assert_eq!(fs::read(&fixture.path).expect("public bytes"), original);
    assert_eq!(fixture.stages(), 0);
}

#[test]
fn root_replacement_rejects_reader_and_builder_without_publishing() {
    let fixture = Fixture::new();
    let database = fixture.save(1);
    let staged = fixture.open();
    let original = fs::read(&fixture.path).expect("public bytes");
    fs::rename(&fixture.root, fixture.parent.join("PreviousRoot")).expect("move old root");
    fs::create_dir(&fixture.root).expect("replacement root");
    assert!(database.check_root().is_err());
    assert!(staged.publish(&Cancellation::default()).is_err());
    assert_eq!(fs::read(&fixture.path).expect("public bytes"), original);
    drop(database);
    assert_eq!(fixture.stages(), 0);
}

#[test]
fn linked_or_shared_cache_files_are_rejected_and_left_intact() {
    let fixture = Fixture::new();
    drop(fixture.save(1));
    let bytes = fs::read(&fixture.path).expect("public bytes");
    let other = fixture.parent.join("unrelated");
    fs::write(&other, b"unrelated data").expect("unrelated file");
    fs::remove_file(&fixture.path).expect("remove fixture cache");
    symlink(&other, &fixture.path).expect("cache symlink");
    assert!(Staged::open(&fixture.path, &fixture.root, &Cancellation::default()).is_err());
    assert_eq!(
        fs::read(&other).expect("unrelated bytes"),
        b"unrelated data"
    );
    fs::remove_file(&fixture.path).expect("remove fixture symlink");
    fs::write(&fixture.path, bytes).expect("restore cache");
    fs::set_permissions(&fixture.path, fs::Permissions::from_mode(0o644)).expect("share fixture");
    assert!(Staged::open(&fixture.path, &fixture.root, &Cancellation::default()).is_err());
    assert_eq!(fixture.stages(), 0);
}

#[test]
fn incompatible_versioned_schema_is_preserved_before_sql_validation() {
    let fixture = Fixture::new();
    drop(fixture.save(1));
    let database = Connection::open(&fixture.path).expect("fixture database");
    let fields = (0..64)
        .map(|number| format!("field{number} TEXT"))
        .collect::<Vec<_>>()
        .join(",");
    database
        .execute_batch(&format!(
            "CREATE TABLE future({fields}); PRAGMA user_version=999;"
        ))
        .expect("future fixture schema");
    drop(database);
    let original = fs::read(&fixture.path).expect("future cache bytes");
    assert!(Staged::open(&fixture.path, &fixture.root, &Cancellation::default()).is_err());
    assert_eq!(
        fs::read(&fixture.path).expect("preserved future cache"),
        original
    );
    assert_eq!(fixture.stages(), 0);
}
