use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{
    action::Action,
    app::{Focus, Mode},
};

pub fn map_key_event(
    key: KeyEvent,
    sequence: &mut Option<char>,
    mode: Mode,
    focus: Focus,
    help_visible: bool,
    overlay_active: bool,
) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Enter {
        return Some(Action::RunQuery);
    }

    if help_visible {
        return match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::F(1) => {
                Some(Action::ToggleHelp)
            }
            _ => None,
        };
    }

    if overlay_active {
        return match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => Some(Action::OverlayCancel),
            (_, KeyCode::Enter) => Some(Action::OverlayAccept),
            (_, KeyCode::Backspace) => Some(Action::OverlayBackspace),
            (modifiers, KeyCode::Char('n')) if modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::OverlayNext)
            }
            (modifiers, KeyCode::Char('p')) if modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::OverlayPrevious)
            }
            (_, KeyCode::Down) => Some(Action::OverlayNext),
            (_, KeyCode::Up) => Some(Action::OverlayPrevious),
            (modifiers, KeyCode::Char(character))
                if modifiers
                    .intersection(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    .is_empty() =>
            {
                Some(Action::OverlayInsert(character))
            }
            _ => None,
        };
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        return Some(Action::SaveQuery);
    }

    if mode == Mode::Insert && key.code == KeyCode::Tab {
        return Some(Action::AcceptCompletion);
    }

    if mode == Mode::Insert && key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('n') => Some(Action::NextCompletion),
            KeyCode::Char('p') => Some(Action::PreviousCompletion),
            _ => None,
        };
    }

    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
        return Some(
            if key.modifiers.contains(KeyModifiers::SHIFT) || key.code == KeyCode::BackTab {
                Action::FocusPrevious
            } else {
                Action::FocusNext
            },
        );
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('n') if focus == Focus::Editor => Some(Action::NewQueryTab),
            KeyCode::Char('n') => Some(Action::MoveDown),
            KeyCode::Char('p') => Some(Action::MoveUp),
            KeyCode::Char('h') | KeyCode::Left => Some(Action::FocusLeft),
            KeyCode::Char('l') | KeyCode::Right => Some(Action::FocusRight),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::FocusUp),
            KeyCode::Char('j') | KeyCode::Down => Some(Action::FocusDown),
            _ => None,
        };
    }

    if mode == Mode::Insert {
        return match key.code {
            KeyCode::Esc => Some(Action::EnterNormalMode),
            KeyCode::Enter => Some(Action::AcceptCompletion),
            KeyCode::Backspace => Some(Action::Backspace),
            KeyCode::Left => Some(Action::MoveLeft),
            KeyCode::Right => Some(Action::MoveRight),
            KeyCode::Up => Some(Action::MoveUp),
            KeyCode::Down => Some(Action::MoveDown),
            KeyCode::Char(character)
                if key
                    .modifiers
                    .intersection(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    .is_empty() =>
            {
                Some(Action::Insert(character))
            }
            _ => None,
        };
    }

    if let Some(pending) = sequence.take() {
        if pending == ' ' && key.code == KeyCode::Char('f') {
            *sequence = Some('f');
            return None;
        }
        if pending == ' ' && key.code == KeyCode::Char('b') {
            *sequence = Some('b');
            return None;
        }
        return match (pending, key.code) {
            (' ', KeyCode::Char('r')) => Some(Action::RunQuery),
            (' ', KeyCode::Char('n')) => Some(Action::NewQueryTab),
            (' ', KeyCode::Char('?')) => Some(Action::ToggleHelp),
            ('f', KeyCode::Char('f')) => Some(Action::OpenSavedQueryFinder),
            ('f', KeyCode::Char('h')) => Some(Action::OpenHistoryFinder),
            ('f', KeyCode::Char('s')) => Some(Action::SaveQueryAs),
            ('b', KeyCode::Char('d')) => Some(Action::RequestCloseQueryTab),
            ('d', KeyCode::Char('d')) => Some(Action::DeleteCurrentLine),
            ('g', KeyCode::Char('g')) => Some(Action::GoToFirstLine),
            ('g', KeyCode::Char('t')) => Some(Action::NextQueryTab),
            ('g', KeyCode::Char('T')) => Some(Action::PreviousQueryTab),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char(' ') => {
            *sequence = Some(' ');
            None
        }
        KeyCode::Char('i') => Some(Action::EnterInsertMode),
        KeyCode::Char('o') => Some(Action::OpenLineBelow),
        KeyCode::Char('O') => Some(Action::OpenLineAbove),
        KeyCode::Char('a') => Some(Action::AppendAfterCursor),
        KeyCode::Char('A') => Some(Action::AppendLineEnd),
        KeyCode::Char('I') => Some(Action::InsertLineStart),
        KeyCode::Char('x') => Some(Action::DeleteCharacter),
        KeyCode::Char('0') => Some(Action::MoveLineStart),
        KeyCode::Char('^') => Some(Action::MoveFirstNonBlank),
        KeyCode::Char('$') => Some(Action::MoveLineEnd),
        KeyCode::Char('w') => Some(Action::MoveWordForward),
        KeyCode::Char('b') => Some(Action::MoveWordBackward),
        KeyCode::Char('e') => Some(Action::MoveWordEnd),
        KeyCode::Char('G') => Some(Action::GoToLastLine),
        KeyCode::Char('[') => Some(Action::PreviousInspectorSection),
        KeyCode::Char(']') => Some(Action::NextInspectorSection),
        KeyCode::Char('p') => Some(Action::PreviewInspectedTable),
        KeyCode::Esc => Some(Action::CloseInspector),
        KeyCode::Char('g') => {
            *sequence = Some('g');
            None
        }
        KeyCode::Char('d') => {
            *sequence = Some('d');
            None
        }
        KeyCode::Char('1') => Some(Action::FocusExplorer),
        KeyCode::Char('2') => Some(Action::FocusEditor),
        KeyCode::Char('3') => Some(Action::FocusResults),
        KeyCode::Enter => Some(Action::Activate),
        KeyCode::Char('?') | KeyCode::F(1) => Some(Action::ToggleHelp),
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Left | KeyCode::Char('h') => Some(Action::MoveLeft),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::MoveDown),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::MoveUp),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::MoveRight),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leader_r_runs_query_in_normal_mode() {
        let mut sequence = None;
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &mut sequence,
                Mode::Normal,
                Focus::Editor,
                false,
                false,
            ),
            None
        );
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
                &mut sequence,
                Mode::Normal,
                Focus::Editor,
                false,
                false,
            ),
            Some(Action::RunQuery)
        );
    }

    #[test]
    fn printable_keys_insert_only_in_insert_mode() {
        let mut sequence = None;
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                &mut sequence,
                Mode::Insert,
                Focus::Editor,
                false,
                false,
            ),
            Some(Action::Insert('x'))
        );
    }

    #[test]
    fn control_n_selects_next_completion() {
        let mut sequence = None;
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                &mut sequence,
                Mode::Insert,
                Focus::Editor,
                false,
                false,
            ),
            Some(Action::NextCompletion)
        );
    }

    #[test]
    fn dd_deletes_current_line() {
        let mut sequence = None;
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                &mut sequence,
                Mode::Normal,
                Focus::Editor,
                false,
                false,
            ),
            None
        );
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                &mut sequence,
                Mode::Normal,
                Focus::Editor,
                false,
                false,
            ),
            Some(Action::DeleteCurrentLine)
        );
    }

    #[test]
    fn control_enter_runs_query() {
        let mut sequence = None;
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
                &mut sequence,
                Mode::Normal,
                Focus::Editor,
                false,
                false,
            ),
            Some(Action::RunQuery)
        );
    }

    #[test]
    fn control_s_saves_query_from_insert_mode() {
        let mut sequence = None;
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                &mut sequence,
                Mode::Insert,
                Focus::Editor,
                false,
                false,
            ),
            Some(Action::SaveQuery)
        );
    }

    #[test]
    fn leader_f_opens_saved_queries_and_history() {
        for (last_key, expected) in [
            ('f', Action::OpenSavedQueryFinder),
            ('h', Action::OpenHistoryFinder),
            ('s', Action::SaveQueryAs),
        ] {
            let mut sequence = None;
            for key in [' ', 'f'] {
                assert_eq!(
                    map_key_event(
                        KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                        &mut sequence,
                        Mode::Normal,
                        Focus::Editor,
                        false,
                        false,
                    ),
                    None
                );
            }
            assert_eq!(
                map_key_event(
                    KeyEvent::new(KeyCode::Char(last_key), KeyModifiers::NONE),
                    &mut sequence,
                    Mode::Normal,
                    Focus::Editor,
                    false,
                    false,
                ),
                Some(expected)
            );
        }
    }

    #[test]
    fn overlay_captures_search_input_and_navigation() {
        let mut sequence = None;
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE),
                &mut sequence,
                Mode::Normal,
                Focus::Editor,
                false,
                true,
            ),
            Some(Action::OverlayInsert('u'))
        );
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                &mut sequence,
                Mode::Normal,
                Focus::Editor,
                false,
                true,
            ),
            Some(Action::OverlayNext)
        );
    }

    #[test]
    fn gt_cycles_query_tabs() {
        for (last_key, expected) in [('t', Action::NextQueryTab), ('T', Action::PreviousQueryTab)] {
            let mut sequence = None;
            assert_eq!(
                map_key_event(
                    KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
                    &mut sequence,
                    Mode::Normal,
                    Focus::Editor,
                    false,
                    false,
                ),
                None
            );
            assert_eq!(
                map_key_event(
                    KeyEvent::new(KeyCode::Char(last_key), KeyModifiers::NONE),
                    &mut sequence,
                    Mode::Normal,
                    Focus::Editor,
                    false,
                    false,
                ),
                Some(expected)
            );
        }
    }

    #[test]
    fn control_n_opens_a_buffer_only_from_normal_editor_mode() {
        let mut sequence = None;
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                &mut sequence,
                Mode::Normal,
                Focus::Editor,
                false,
                false,
            ),
            Some(Action::NewQueryTab)
        );
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                &mut sequence,
                Mode::Normal,
                Focus::Explorer,
                false,
                false,
            ),
            Some(Action::MoveDown)
        );
    }

    #[test]
    fn leader_bd_requests_closing_the_current_buffer() {
        let mut sequence = None;
        for key in [' ', 'b'] {
            assert_eq!(
                map_key_event(
                    KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                    &mut sequence,
                    Mode::Normal,
                    Focus::Editor,
                    false,
                    false,
                ),
                None
            );
        }
        assert_eq!(
            map_key_event(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                &mut sequence,
                Mode::Normal,
                Focus::Editor,
                false,
                false,
            ),
            Some(Action::RequestCloseQueryTab)
        );
    }
}
