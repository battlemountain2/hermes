# Saved searches and Smart Folders

Smart Folders are named searches, not real directories. They show current matches while files stay
in their original locations. They reuse the shared search catalog and its existing coverage limits.

## Use

1. Open Search with Ctrl+K, choose Home, Music library or This folder, and set the query/filters.
2. Choose Save search, enter a name and Save. Success appears only after storage acknowledges it.
3. Open the named item under SMART FOLDERS in the sidebar to see its current matches.
4. Manage searches… opens the editor: select an item, edit its name/query/scope/location/results,
   and Save. Load selected restores its current saved version, discarding those editor fields.

All, Files and Songs are saved as physical result sets. For Albums or Artists, first open an entity
in Search, then save its scoped Songs. Album/artist membership stays separate from typed filters
and is preserved when changing the name, query or location. Switching away from Songs asks before
clearing that membership. Album results retain the existing disc/track ordering.

For example, `type:image ext:jpg` in a photo folder finds JPEG images; `type:video modified:today`
finds recently modified videos. Content extraction stays opt-in through the existing content filter.
Other drives use This folder with an absolute local path. Missing/offline roots remain saved.

Open uses the committed saved query, not unsaved editor changes. Delete asks for confirmation and
removes only the saved definition; no result files are removed. Query result views retain the normal
file context menus, preview behavior and incomplete-coverage notices.

## Persistence and recovery

Definitions live in `$XDG_DATA_HOME/strata/saved-searches/queries-v1.json` (default
`~/.local/share/strata/saved-searches/queries-v1.json`), separate from cache databases. Clearing a
search catalog does not remove these definitions. No file tags or original contents are modified.

- Up to128 entries, unique case-folded names of1–80 characters and1MiB serialized data.
- Canonical queries retain the4096-byte query/root limits; requests and worker queues are bounded.
- All disk work runs off the interface thread. Saves commit before they appear in the sidebar.
- A private0700 directory and0600 files, single-writer lock, guarded temporary writes and expected
  file bytes prevent common clobbering. Detected external edits require Reload rather than overwrite.
- Corrupt or newer-version data is preserved, with editing blocked until storage is recovered and
  reloaded. Validation errors such as duplicate names keep fields editable for correction.
- Stale edit/delete requests compare against the exact original definition. Reload preserves editor
  fields; Load selected explicitly replaces them with the current saved entry.
- Shutdown allows at most1s to drain pending work. A hard process kill can lose an unacknowledged
  request; a successful Save is already durable. This is not a general backup or file-tag service.

## Interface pattern

The editor extends the existing settings/search components: semantic theme colors, square search
frame, fixed800×600 layout, scrollable list/editor and reserved status/confirmation/footer slots.
Long names, errors, loading, selection and confirmations do not animate or resize the frame.
Underlying window resizing can still reposition its centered frame.

States include loading, empty, editing, saving, saved, validation error, stale edit and unavailable
storage. Save/delete are disabled during pending commits; Reload retries storage. No result indexing
or metadata decoding is performed by the manager. Hidden managers stop polling; visible managers
and the sidebar read a cheap revision counter before cloning changed snapshots.

Controls reuse theme text/surface/accent/dim/border colors and existing hover/focus/disabled states.
Entries and selectors have accessible labels, status uses a Status role, and Tab stays inside the
modal. Ctrl+S saves, Ctrl+R reloads, Alt+Enter opens saved results and Esc closes. Unsaved editor fields
remain while reopening the same manager; selecting/loading another entry replaces them.

This increment does not add filesystem-mounted virtual folders, grouped album/artist result tabs,
compound rule builders, ratings, larger search coverage or new indexing backends.
