# Custom actions

Hermes can add personal commands to the file and folder context menu. Open
**Settings → Custom actions** to create or edit `~/.config/hermes/actions.toml`.
Restart Hermes after editing the file to refresh its context menus.

Each action uses an argument array. Hermes starts the executable directly and never invokes a
shell, so spaces and shell characters in filenames remain data rather than executable syntax.

```toml
[[actions]]
name = "Edit with Kate"
command = ["kate", "{paths}"]
extensions = ["txt", "md", "toml", "rs"]
multiple = true

[[actions]]
name = "Open terminal here"
command = ["foot", "--working-directory", "{path}"]
directories = true

[[actions]]
name = "Optimize PNG"
command = ["oxipng", "--", "{path}"]
extensions = ["png"]
confirm = true
```

Available placeholders:

- `{path}` — the first selected item's full path
- `{paths}` — every selected path, expanded as separate arguments
- `{parent}` — the first selected item's parent directory
- `{name}` — the first selected item's filename

An empty `extensions` list accepts any file. Set `directories = true` to show the action for
folders, `multiple = true` to allow multi-selection, and `confirm = true` to ask before running it.
Custom actions execute with your normal user permissions, so only configure commands you trust.
