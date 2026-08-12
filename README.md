# PGIDE

A keyboard-first PostgreSQL terminal IDE, currently at its foundation milestone.

## Run

```sh
cargo run
cargo run -- 'postgres://user:password@localhost/database'
```

The connection URL is used only in memory and is never displayed or logged.

## Initial keymap

- `i`: enter insert mode
- `Esc`: return to normal mode
- `<leader>r` (Space, then `r`): execute the SQL buffer
- `?` or `F1`: open the in-app cheat sheet
- `Tab` / `Shift-Tab`: cycle Explorer, SQL, and Results
- `Ctrl-h/j/k/l`: focus left/down/up/right panel
- `1` / `2` / `3`: jump to Explorer / SQL / Results
- `Enter`: expand or collapse the focused Explorer node
- Mouse click: focus panels and toggle Explorer nodes
- `h/j/k/l`: navigate result cells when Results is active
- `Tab` in INSERT mode: accept the highlighted SQL completion
- `Ctrl-n` / `Ctrl-p`: select the next / previous completion
- `Enter` in INSERT mode: accept completion, or insert a newline when none is shown
- `Ctrl-Enter` or `<leader>r`: run the statement under the cursor and focus Results
- `o`: open a new line below and enter INSERT mode
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
connected database, ranked with Nucleo's fuzzy matcher.

The Results panel supports selected-cell navigation with `h/j/k/l`, horizontal column paging,
vertical row scrolling, row numbers, NULL styling, and native rendering for UUID, date/time,
JSON/JSONB, boolean, numeric, text, enum, and bytea values.
- `Ctrl-C` or `q` in normal mode: quit
- Arrow keys: move the editor cursor
- Backspace/Enter: edit while in insert mode
