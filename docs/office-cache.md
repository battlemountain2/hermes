# Persistent converted Office PDF cache

`src/sandbox/office/disk.rs` stores complete converted PDFs across application
restarts. It is an optional accelerator, never a replacement for sandboxed
conversion or PDF validation/rendering. It does not cache extracted text, posters,
first-page thumbnail exports, user profiles, or original Office documents.

## Integration contract

Declare `mod disk;` in `src/sandbox/office.rs`. The API is private to that module:

```rust
let captured = disk::key(input).ok();
if let Some(bytes) = captured
    .as_ref()
    .and_then(|key| disk::load(key, cancellation))
{
    return Ok(bytes);
}
let canonical = captured.as_ref().map_or(input, |key| key.input());
let bytes = convert(canonical)?;
if let Some(key) = captured.as_ref() {
    let _saved = disk::store(key, &bytes, cancellation);
}
Ok(bytes)
```

- `key(input: &Path) -> Result<Key, String>` captures the identity **before**
  conversion. Do not capture a replacement key after converting: that can label
  an old PDF with a new source's identity.
- `load(&Key, &Cancellation) -> Option<Vec<u8>>` rechecks the captured source and
  rendering environment before and after reading. All failures are cache misses.
- `store(&Key, &[u8], &Cancellation) -> Result<(), String>` rechecks the captured
  identity before staging, before publication, and immediately after publication.
  A detected change or cancellation never leaves the newly published record.
- `Key` is opaque, `Clone + Debug + Eq + PartialEq`. `Key::input() -> &Path`
  supplies the captured canonical conversion path. `Key::is_current(cancellation)
  is available for memory-cache admission checks.

Keep the parent's existing cancellation/source guards around returned page data,
including memory-cache hits. Include the captured disk key in the memory identity
so renderer/font changes invalidate both cache layers. If key construction fails,
the parent may still convert using its existing canonical source identity and
guards; it must not use an unvalidated disk record. Disk I/O stays on the existing
preview worker, never the GTK main thread. No background writer or delayed
recapture of the source is needed.

The schema covers the normal full-PDF writer/impress/calc export. Source package
autodetection selects the export filter; documents and workbook Layout previews
can share this cache. If export settings, profile policy, locale, sandbox mounts,
format policy, or workbook print mode change, update `CONFIG`/the schema. Do not
use this key for a first-page or alternate-mode export without a distinct config.
The current CONFIG revision is office-full-pdf-v2, with embedded raster images capped at150dpi;
text/vectors remain unflattened. Earlier uncapped PDFs are misses rather than reused under new settings.

## Identity and invalidation

Keys contain the canonical path's **raw Unix bytes**, device, inode, size,
mtime/ctime seconds and nanoseconds, mode, and SHA-256 of the render environment.
Restoring mtime alone after a same-length edit does not preserve the identity.
Sources must be regular files no larger than 50 MiB. Conversion uses the captured
canonical path, not an alias that can be retargeted during conversion.

The environment fingerprint uses the existing fixed renderer discovery, not
`PATH`, and the running helper executable's `/proc/self/exe` metadata. It captures
the resolved renderer, its program directory, `soffice.bin`, `libmergedlo.so`,
bootstrap/fundamental/version configuration, and bundled registry/config/filter/
font trees. Fixed `/usr/lib/libreoffice`, `/usr/lib64/libreoffice`, and
`/opt/libreoffice` runtime locations are covered even when discovery selects a
`/usr/bin` wrapper; the dynamic-linker cache is included as well.
Rebuilding the application invalidates records intentionally; restarting
the same installed build does not.

Font metadata roots are `/etc/fonts`, `/usr/share/fonts`, `/usr/local/share/fonts`,
`/usr/share/fontconfig`, `/usr/lib/fontconfig`, and `/var/cache/fontconfig`. This
matches system fonts/configuration rather than personal fonts unavailable to the
isolated helper. Traversal is deterministic, metadata-only, cycle-deduplicated,
and bounded to 8,192 visited entries/24 levels. Missing optional roots are part of
the fingerprint; unreadable or over-budget trees disable caching. Nested font,
fontconfig, cache, renderer-binary and configuration changes invalidate records.
No version command, shell, font-cache refresh, upload, or network operation runs.
There is no TTL window that can incorrectly accept a recently changed environment.
Warm-path traversal cost has not been measured; the parent owns performance
measurement and any shared build/test runs.

As with the existing source identity, metadata revalidation detects changes at
the validation boundaries; it does not freeze a concurrently writable source or
renderer tree. A mutation after the final check is handled by the parent's outer
source/cancellation checks. SHA-256 is corruption detection, not authentication
against an attacker already able to execute as the same Unix user.

## Storage and safety

- Root: `glib::user_cache_dir()/strata/office-pdf-v1`.
- Cache root must already be owned by the current user with exactly `0700`, or
  is created with `0700`. Existing unsafe permissions are rejected, never repaired.
  All path components are opened with `O_DIRECTORY | O_NOFOLLOW`, with operations
  anchored to the pinned directory handle through `/proc/self/fd`.
- Records/staging files are owned, single-link regular files with exactly `0600`.
  `O_NOFOLLOW | O_NONBLOCK` rejects symlinks without blocking on a FIFO. Symlinked,
  hardlinked, special, oversized, foreign-owned, or nonprivate records are not used.
- A nonblocking directory lock serializes cooperating processes. Contention is a
  miss/store error, never a main-thread wait or an unsafe fallback.
- Each envelope carries a version magic, bounded JSON header, complete captured
  key, exact PDF length and SHA-256. PDFs must be at most 32 MiB, have a versioned
  `%PDF-` header and terminal `%%EOF`. Invalid envelopes, mismatched identities,
  truncation, checksum failures and trailing bytes are misses. This is not a full
  PDF parser: bytes still go through the existing isolated PDF renderer.
- Publication uses an exclusively created `0600` temporary file, file sync,
  same-directory atomic rename, post-publication identity check, and directory
  sync. Failed writes remove staging files. A later writer reclaims private orphan
  staging files after a crash; unrelated names are left untouched.
- LRU uses persisted record modification times, updated only after a successful
  validated hit. Pruning bounds committed envelopes to **8 entries / 128 MiB**,
  including envelope overhead. Atomic replacement may temporarily retain the old
  record plus one staged record, bounded by one additional envelope (32 MiB plus
  at most 128 KiB header). Directory enumeration stops at 64 entries. Unsafe owned
  names disable writes rather than following or modifying their targets.

The cache contains private rendered document content, so normal local filesystem
privacy applies; it is not encrypted. It changes only files in its own cache
subdirectory. It never modifies original Office files or the user's font caches.

## Verification

Adjacent tests live in `src/sandbox/office/disk/tests.rs`, declared by the disk
module. They use private generated temporary directories and synthetic sources,
PDF bytes and rendering/font environments, never personal files or a user GUI.
Coverage includes restart-independent reads, source replacement/ctime changes,
cancellation and post-publication rejection, corruption/length/schema checks,
symlink/hardlink/permission defenses, lock contention, LRU/count/byte budgets,
orphan staging cleanup, non-UTF-8 paths and deterministic environment invalidation.

The ignored `sandbox::office::disk::tests::persistent_cache_survives_a_fresh_process`
case starts the same test binary in a fresh process and reads a generated cache.
Run it explicitly with `--ignored --exact` under the parent's coordinated test
build. Implementation here did not run Cargo, the shared build, or user GUI/audio;
test execution and integration validation belong to the parent.

Parent-reported integration validation: 31 Office unit tests pass, including the
compiled disk-cache tests and initial load/store round trip. The actual workbook
adapter's persisted Layout PDF cache and Layout/Data toggle guards also pass;
the fast-poster path was measured at 81 ms. These are parent-reported results,
not a separate test run by this cache implementation task.
The renderer/font key also participates in the parent's memory identity. Coordinated follow-up
passed the explicit fresh-process regression and measured the reported 49-slide deck at about
12.49 seconds for a new conversion versus 1.12 seconds in a fresh process using the persisted PDF.
Both runs used the same debug build and a private test cache on this machine; this is not a cold
disk, all-documents or release-build speed guarantee. Fingerprint/PDF-render cost is included.
