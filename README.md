# Mantra

Mantra is a keyboard-first PostgreSQL terminal IDE: navigate a database like a file tree, write SQL
like code, and inspect results without leaving the terminal.

The name is intentionally broader than a driver or a single screen. The product goal is a calm,
repeatable PostgreSQL workflow that stays close to the keyboard while remaining usable with a
mouse.

## Why this exists

I wanted a PostgreSQL workspace that matched the way I already work in a terminal: an explorer on
the left, a real editor in the middle, and a result surface that can be searched, copied, and
eventually edited in place. I tried established terminal clients such as [pgcli](https://github.com/dbcli/pgcli)
and [tsql](https://github.com/fcoury/tsql). They are useful references, but my workflow needed the
database tree, saved SQL buffers, schema-aware completion, query history, and result navigation to
live together in one focused workspace.

Mantra is being built for that use case first. It is not trying to be every database tool; it is
trying to make PostgreSQL work from a terminal feel deliberate and dependable.

## Installation

Mantra is in active pre-release development. Install the latest release with Homebrew:

```sh
brew install Nabwinsaud/tap/mantra
```

The formula works on macOS and Linux. Debian and Ubuntu users can instead use the signed APT
repository, and a checksum-verifying installer is available for macOS, Debian, and Ubuntu:

```sh
curl -fsSL https://raw.githubusercontent.com/Nabwinsaud/mantra-packages/main/install.sh | sh
```

See [Mantra Packages](https://github.com/Nabwinsaud/mantra-packages) for signed APT setup,
repository keys, supported architectures, and packaging details.

## Run

To run from a source checkout:

```sh
cargo run
cargo run -- 'postgres://user:password@localhost/database'
```

The connection URL is used only in memory and is never displayed or logged.

## Initial keymap

- `i`: enter insert mode
- `Esc`: return to normal mode
- `<leader>r` (Space, then `r`): execute the SQL buffer
- `Space`: open a LazyVim-style which-key panel for available leader commands
- `?` or `F1`: open the in-app cheat sheet
- `Tab` / `Shift-Tab`: cycle Explorer, SQL, and Results
- `Ctrl-h/j/k/l`: focus left/down/up/right panel
- `1` / `2` / `3`: jump to Explorer / SQL / Results
- `Enter`: expand or collapse the focused Explorer node
- Mouse click: focus panels and toggle Explorer nodes
- `h/j/k/l`: navigate result cells when Results is active
- `y` / `yy` in Results: copy the selected cell / selected row as TSV
- `e` in Results: open a primary-key-scoped `UPDATE` template for the selected cell
- `dd` in Results: confirm, then open a primary-key-scoped `DELETE` in a new SQL tab
- `yy` in Table Inspector: copy the complete table schema as shareable Markdown
- `ya` in Table Inspector: copy the schema with an AI prompt requesting 20 sample rows
- `Tab` in INSERT mode: accept the highlighted SQL completion
- `Ctrl-n` / `Ctrl-p`: select the next / previous completion
- `Enter` in INSERT mode: accept completion, or insert a newline when none is shown
- `Ctrl-Enter` or `<leader>r`: run the statement under the cursor and focus Results
- `Ctrl-s`: save the current query; the first save asks for a name
- `Ctrl-n` in the NORMAL SQL editor, or `<leader>n`: open a new empty query tab
- `<leader>bd`: close the current tab after confirmation without deleting its saved query
- `<leader>ff`: fuzzy-find saved queries for the current project and database
- `<leader>fh`: fuzzy-find automatically recorded query history
- `<leader>ft`: fuzzy-find a table and open its inspector
- `<leader>e`: toggle the Explorer sidebar
- `<leader>fs`: save the current SQL as a new query instead of updating the opened one
- `gt` / `gT`: move to the next / previous SQL buffer tab
- `<leader>bn` / `<leader>bp`: move to the next / previous SQL buffer tab
- `o`: open a new line below and enter INSERT mode
- `u` / `Ctrl-r` in the NORMAL SQL editor: undo / redo changes in the current buffer
- `dd`: delete the current line
- `a` / `A`: append after cursor / at line end
- `I`: insert at the first non-blank character
- `o` / `O`: open a line below / above
- `x`: delete the character under the cursor
- `0` / `^` / `$`: line start / first non-blank / line end
- `w` / `b` / `e`: next word / previous word / word end
- `gg` / `G`: first line / last line

Execution is cursor-aware: in a buffer containing several semicolon-separated statements,
`Ctrl-Enter` and `<leader>r` execute only the statement containing the cursor. Semicolons inside SQL
strings and comments are handled by the PostgreSQL tokenizer.

The SQL editor uses PostgreSQL-aware tokenization for syntax colors. Completion combines SQL
keywords with schemas, tables, columns, `schema.table`, and `table.column` names loaded from the
connected database, ranked with Nucleo's fuzzy matcher. SQL aliases are scoped to their source
table, so `FROM user_roles ur ... ur.` completes as `ur."userId"`, not an unrelated table's
column. Mixed-case PostgreSQL identifiers are quoted automatically to prevent case folding.

The editor includes a line-number gutter, current-line highlighting, cursor-following vertical
scrolling, mode-colored statusline, and contextual which-key panels for leader and multi-key
commands.

## Saved queries and history

Press `Ctrl-s` to name and save the current SQL buffer. Later `Ctrl-s` updates the same saved
query. Saved queries are scoped to both the directory where Mantra was started and the connected
database. A portable SQL copy is written to `.pgide/queries/`, while searchable metadata and query
history are stored in Mantra's platform-local SQLite database. The `.pgide` path is retained for
compatibility with workspaces created before the product was renamed to Mantra.

- `<leader>ff` (Space, `f`, `f`): open saved queries
- `<leader>fh` (Space, `f`, `h`): open query history
- `<leader>fs` (Space, `f`, `s`): save as a new query
- Type to fuzzy-filter, use `Ctrl-n` / `Ctrl-p` or arrow keys to select, then press `Enter`
- Press `Esc` to close either finder

Every query sent to PostgreSQL is recorded with its success state, execution time, database, and
timestamp. Connection strings and credentials are never stored in query history.

Saved queries open as file-backed editor tabs. Opening the same saved query again focuses its
existing tab, and `Ctrl-s` updates that file without asking for its name again. A `●` beside a tab
means its buffer has changes that have not been saved. Open saved tabs and the active tab are
restored the next time the same project and database are opened; click a tab or use `gt` / `gT` to
switch between them. If an older workspace has saved queries but no tab session yet, Mantra opens
the most recently saved query automatically.

`<leader>bd` (Space, `b`, `d`) closes only the editor tab. Mantra always asks first, including for
unnamed scratch buffers. Press `y` or click **Close tab** to confirm; press `n`, `Esc`, or click
**Cancel** to keep it open. Saved-query records and `.sql` files are never deleted by this action.

The Results panel supports selected-cell navigation with `h/j/k/l`, horizontal column paging,
vertical row scrolling, row numbers, NULL styling, and native rendering for UUID, date/time,
JSON/JSONB, boolean, numeric, text, enum, and bytea values.

## Table inspector

Focus the Explorer, select a table, and press `Enter` (or click the table) to lazily load its
PostgreSQL metadata. The inspector shows columns, native types, nullability, defaults, comments,
keys, constraints, index definitions, estimated rows, and storage sizes without loading every
table at startup.

- `h/l` or `[/]`: switch inspector sections
- `p`: preview the first 100 rows with a safely quoted generated query
- `Esc`: close the inspector and return to the Explorer
- `Ctrl-C` or `q` in normal mode: quit
- Arrow keys: move the editor cursor
- Backspace/Enter: edit while in insert mode
