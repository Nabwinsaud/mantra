mod action;
mod app;
mod database;
mod event;
mod sql;
mod storage;
mod terminal;
mod ui;

use std::{env, io};

use anyhow::Result;
use app::App;
use crossterm::event::{Event as CrosstermEvent, EventStream, MouseButton, MouseEventKind};
use event::map_key_event;
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let connection_url = env::args().nth(1);
    let mut terminal = terminal::TerminalGuard::new(io::stdout())?;
    let (database, mut database_events) = database::DatabaseService::spawn();
    let project_root = env::current_dir()?;
    let storage = storage::Storage::open(&project_root)?;
    let mut app = App::with_storage(database, storage);
    let mut rendered_mode = None;

    if let Some(url) = connection_url {
        app.connect(url).await;
    }

    let mut input = EventStream::new();
    while !app.should_quit {
        if rendered_mode != Some(app.mode) {
            terminal.set_cursor_style(app.mode == app::Mode::Insert)?;
            rendered_mode = Some(app.mode);
        }
        terminal.draw(|frame| ui::draw(frame, &app))?;

        tokio::select! {
            maybe_event = input.next() => {
                match maybe_event {
                    Some(Ok(CrosstermEvent::Key(key))) => {
                        let overlay_active = app.overlay_active();
                        if let Some(action) = map_key_event(
                            key,
                            &mut app.key_sequence,
                            app.mode,
                            app.focus,
                            app.help_visible,
                            overlay_active,
                        ) {
                            app.update(action).await;
                        }
                    }
                    Some(Ok(CrosstermEvent::Paste(text))) if app.mode == app::Mode::Insert => {
                        app.update(action::Action::Paste(text)).await;
                    }
                    Some(Ok(CrosstermEvent::Resize(_, _))) => {}
                    Some(Ok(CrosstermEvent::Mouse(mouse)))
                        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) =>
                    {
                        let (width, height) = crossterm::terminal::size()?;
                        let action = ui::mouse_action(mouse.column, mouse.row, width, height, &app);
                        app.update(action).await;
                    }
                    Some(Err(error)) => return Err(error.into()),
                    None => break,
                    _ => {}
                }
            }
            Some(event) = database_events.recv() => app.handle_database_event(event),
        }
    }

    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .init();
}
