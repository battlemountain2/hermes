# Agent Instructions

## Current handoff

- Read `NEXT_AGENT.md` for the latest user priorities and the status of the Antigravity work-in-progress snapshot.

## Git workflow

- Never commit or push directly to `main`. Work from a GitHub issue and submit changes through a pull request.
- Name branches `<type>/<issue-number>-<short-kebab-description>`, for example `feat/6-sandbox-previews`. Use Conventional Commit types such as `feat`, `fix`, `docs`, `refactor`, `test`, `perf`, `build`, `ci`, and `chore`.
- Write commits and pull request titles in Conventional Commits format: `<type>(optional-scope): <imperative description>`.
- Keep commits focused. Use `!` and a `BREAKING CHANGE:` footer for breaking changes, and reference the issue in the pull request body.

## Test organization

- Do not place test implementations inline with production code.
- Put module unit tests in an adjacent test module, such as `src/app/navigation/tests.rs`, and declare it from the implementation with `#[cfg(test)] mod tests;`.
- Use the top-level `tests/` directory for integration tests that exercise the crate through its public API.

## Icons

- Add new interface icons only from the Lucide icon set.
- Keep Lucide geometry intact, namespace bundled assets with `strata-`, and preserve the ISC attribution in `THIRD_PARTY_LICENSES.md`.
- Render theme-colored bundled icons through `assets::primary_icon` / `assets::set_primary_icon`; direct icon-theme loading preserves the SVG's fallback color and will not follow live theme changes.

## Theming

- Apply semantic `@theme_*` colors to every visual state of new interface elements, including icons, text, backgrounds, borders, focus rings, selections, hover/active states, menus, and dialogs.
- Never use static hex/RGB colors for themeable interface elements. Built-in, custom, and Omarchy themes must remain visually consistent and update live.
