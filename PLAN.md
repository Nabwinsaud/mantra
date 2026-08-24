# Mantra release plan

This is the working checklist for the first public release. Checked items describe behavior that
already exists; unchecked items are deliberately still work.

## First public release

### Foundation

- [x] Rust 2024 application with a responsive Ratatui/Crossterm event loop
- [x] PostgreSQL connection and asynchronous query execution
- [x] Cursor-aware execution for the statement under the cursor
- [x] Result table with row/column navigation and PostgreSQL value rendering
- [x] Explorer with lazy schema, table, view, and function loading
- [x] Table inspector with columns, constraints, indexes, and storage details
- [x] Vim-style NORMAL and INSERT editing modes
- [x] SQL syntax highlighting and schema-aware completion
- [x] Saved queries, query history, fuzzy finders, and file-backed SQL tabs
- [x] Confirmation before closing a query tab
- [ ] Stable keyboard behavior in common terminal and tmux configurations
- [ ] Configurable keymaps and a complete command palette
- [ ] Production-connection indicator and destructive-query safeguards
- [ ] User-facing configuration and theme support
- [ ] End-to-end test database and documented development setup

### Distribution

The only supported installation path today is `cargo run` from a checkout. Packaging is pending:

The channels will be delivered incrementally but synchronized by one tagged-release pipeline. See
[RELEASING.md](RELEASING.md) for the artifact contract, registry rules, rollout phases, and release
gates.

- [ ] Add CI gates for format, Clippy, tests, terminal smoke tests, and package metadata
- [ ] Build signed/checksummed macOS and Linux release artifacts from SemVer tags
- [ ] Publish a release manifest and provenance with each tagged GitHub Release
- [ ] Publish and automatically synchronize a Homebrew tap (`brew install mantra`)
- [ ] Publish and synchronize an AUR package with a protected approval gate (validated `PKGBUILD` added)
- [x] Publish automatically validated `amd64` and `arm64` Debian packages (`.deb`)
- [ ] Publish and atomically synchronize a signed APT repository and keyring package
- [ ] Add a checksum-verifying curl installer for macOS and Linux
- [ ] Add post-release install, upgrade, uninstall, retry, and cross-channel drift tests
- [ ] Add shell completions and a `mantra(1)` manual page
- [ ] Document upgrades, uninstall, configuration paths, and credential safety
- [x] Keep package metadata, binary name, README, and screenshots consistent with **Mantra**

## Result editing plan

Results are read-only today. Editing should be explicit, transactional, and safe rather than
turning a table view into an unreviewable mutation tool.

### Navigation and selection

- [ ] Keep `h/j/k/l`, arrows, paging, and virtual rendering fast for large results
- [ ] Select a cell, row, column, or all visible rows
- [ ] Show the selected row, column, table, and connection clearly in the status line
- [ ] Support a record/detail view for wide rows
- [ ] Preserve selection while refreshing or paging results

### Copy and yank

- [x] `y` yank the selected cell
- [x] `yy` yank the selected row as TSV
- [x] `yy` in Table Inspector yank AI/human-readable Markdown schema context
- [x] `ya` in Table Inspector yank schema context with a sample-data prompt
- [ ] `yc` yank the selected column
- [ ] `ya` yank all visible rows
- [ ] Choose plain text, TSV, CSV, JSON, Markdown, or SQL `INSERT` output
- [ ] Use the system clipboard and OSC 52 when the terminal supports it
- [ ] Show a confirmation/status message with the copied shape and row count

### Safe mutation

- [x] `e` generate a primary-key-scoped `UPDATE` template for the selected cell
- [ ] Toggle a cell between a value and `NULL` without confusing the two
- [ ] `i` insert a row with defaults, required fields, enums, and foreign-key hints
- [x] `dd` request deletion of the selected row, then require confirmation in a modal
- [x] Build generated `UPDATE`/`DELETE` predicates from PostgreSQL primary-key metadata
- [x] Refuse ambiguous generated mutations when no safe key exists
- [x] Show generated mutation SQL before execution
- [ ] Run edits inside an explicit transaction with commit and rollback controls
- [ ] Show affected-row counts and detect unexpected multi-row changes
- [ ] Handle permission errors, constraint errors, conflicts, and stale rows clearly
- [ ] Support undo/rollback for the current edit session before commit

### Bulk workflows

- [ ] Multi-select rows for update, delete, copy, and export
- [ ] Apply a value to a selected column with a preview
- [ ] Paginate or stream edits without loading the entire table into memory
- [ ] Keep all dangerous operations visibly marked for production connections

## Editor and UX backlog

- [x] Add fuzzy table search (`<leader>ft`) and Explorer sidebar toggle (`<leader>e`)
- [x] Support Vim/LazyVim-style query-buffer navigation (`gt`/`gT`, `<leader>bn`/`bp`)
- [ ] Make focus and mode unmistakable in every panel
- [ ] Add mouse-clickable command palette and keymap help
- [ ] Add terminal capability detection and reliable fallbacks for Ctrl/Alt shortcuts
- [ ] Make tmux pane-navigation conflicts discoverable in the help screen
- [ ] Improve completion ranking, alias scope, function signatures, and JOIN suggestions
- [ ] Add formatting and diagnostics as opt-in editor commands
- [x] Add per-buffer undo/redo history (`u` / `Ctrl-r`)
- [ ] Add search/replace and multiple selections incrementally
- [ ] Improve wide-result rendering with column pinning and horizontal scroll indicators
- [ ] Add CSV, JSON, Markdown, and SQL export commands
- [ ] Add EXPLAIN/EXPLAIN ANALYZE plan viewing
- [ ] Add transaction, session, lock, and active-query panels
- [ ] Add connection profiles without storing plaintext passwords
- [ ] Add integration tests against disposable PostgreSQL versions
- [ ] Benchmark completion, schema loading, history search, and large result rendering

## Release principles

- [ ] Keep PostgreSQL work off the render loop
- [ ] Never execute generated mutation SQL without an explicit confirmation
- [ ] Never log or persist database passwords
- [ ] Prefer small, reviewable changes with focused commits
- [ ] Keep the terminal workflow useful even when optional capabilities are unavailable
