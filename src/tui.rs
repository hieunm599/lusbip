use std::io::{Write, stdout};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode, size,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionAction {
    Up,
    Down,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKeyAction {
    Up,
    Down,
    Activate,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiItem {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessingIndicator {
    index: usize,
    frame: usize,
}

pub fn next_index(current: usize, len: usize, action: SelectionAction) -> usize {
    if len == 0 {
        return 0;
    }

    match action {
        SelectionAction::Up => (current + len - 1) % len,
        SelectionAction::Down => (current + 1) % len,
        SelectionAction::Toggle => current.min(len - 1),
    }
}

pub fn should_flush_startup_event(event: &Event) -> bool {
    matches!(event, Event::Key(_))
}

pub fn list_key_action(event: &Event) -> Option<ListKeyAction> {
    match event {
        Event::Key(key) => match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(ListKeyAction::Up),
            KeyCode::Down | KeyCode::Char('j') => Some(ListKeyAction::Down),
            KeyCode::Char(' ') => Some(ListKeyAction::Activate),
            KeyCode::Esc => Some(ListKeyAction::Cancel),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(ListKeyAction::Cancel)
            }
            _ => None,
        },
        _ => None,
    }
}

pub fn spinner_frame(frame: usize) -> &'static str {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    FRAMES[frame % FRAMES.len()]
}

pub fn label_with_spinner(label: &str, frame: usize) -> String {
    format!("{label}  {}", spinner_frame(frame))
}

pub fn truncate_to_width(value: &str, width: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width <= 3 {
        return ".".repeat(width);
    }

    let prefix = value.chars().take(width - 3).collect::<String>();
    format!("{prefix}...")
}

pub fn run_action_list<LoadItems, Activate, Exit>(
    title: &str,
    mut load_items: LoadItems,
    mut activate: Activate,
    mut exit: Exit,
) -> Result<(), String>
where
    LoadItems: FnMut() -> Result<Vec<TuiItem>, String>,
    Activate: FnMut(usize) -> Result<String, String> + Send,
    Exit: FnMut() -> Result<(), String> + Send,
{
    let mut terminal = TerminalGuard::enter()?;
    let mut selected = 0usize;
    let mut message: Option<String> = None;

    loop {
        let items = load_items()?;
        if !items.is_empty() {
            selected = selected.min(items.len() - 1);
        } else {
            selected = 0;
        }
        draw_items(
            title,
            &items,
            selected,
            &[],
            false,
            message.as_deref(),
            None,
        )?;
        if !event::poll(Duration::from_millis(1000))
            .map_err(|err| format!("Failed to poll terminal event: {err}"))?
        {
            continue;
        }

        match event::read().map_err(|err| format!("Failed to read terminal event: {err}"))? {
            event if matches!(list_key_action(&event), Some(ListKeyAction::Up)) => {
                selected = next_index(selected, items.len(), SelectionAction::Up);
            }
            event if matches!(list_key_action(&event), Some(ListKeyAction::Down)) => {
                selected = next_index(selected, items.len(), SelectionAction::Down);
            }
            event if matches!(list_key_action(&event), Some(ListKeyAction::Activate)) => {
                if items.is_empty() {
                    continue;
                }
                let result = run_with_spinner(title, &items, selected, || activate(selected))?;
                message = result.starts_with("Error:").then_some(result);
            }
            event if matches!(list_key_action(&event), Some(ListKeyAction::Cancel)) => {
                message = Some(run_with_spinner(title, &items, selected, || {
                    exit().map(|()| "Detached attached USB/IP ports".to_string())
                })?);
                if !message.as_deref().unwrap_or_default().starts_with("Error:") {
                    return Ok(());
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
        terminal.touch();
    }
}

pub fn select_one(title: &str, items: &[TuiItem], _help: &str) -> Result<usize, String> {
    if items.is_empty() {
        return Err("No items to select".into());
    }

    let mut terminal = TerminalGuard::enter()?;
    let mut selected = 0usize;

    loop {
        draw_items(title, items, selected, &[], false, None, None)?;
        match event::read().map_err(|err| format!("Failed to read terminal event: {err}"))? {
            Event::Key(key) => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = next_index(selected, items.len(), SelectionAction::Up);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = next_index(selected, items.len(), SelectionAction::Down);
                }
                KeyCode::Enter => return Ok(selected),
                KeyCode::Esc => return Err("Selection cancelled".into()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("Selection cancelled".into());
                }
                _ => {}
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
        terminal.touch();
    }
}

pub fn select_many(title: &str, items: &[TuiItem], _help: &str) -> Result<Vec<usize>, String> {
    if items.is_empty() {
        return Err("No items to select".into());
    }

    let mut terminal = TerminalGuard::enter()?;
    let mut selected = 0usize;
    let mut chosen = Vec::<usize>::new();

    loop {
        draw_items(title, items, selected, &chosen, true, None, None)?;
        match event::read().map_err(|err| format!("Failed to read terminal event: {err}"))? {
            Event::Key(key) => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = next_index(selected, items.len(), SelectionAction::Up);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = next_index(selected, items.len(), SelectionAction::Down);
                }
                KeyCode::Char(' ') => {
                    if let Some(pos) = chosen.iter().position(|idx| *idx == selected) {
                        chosen.remove(pos);
                    } else {
                        chosen.push(selected);
                    }
                }
                KeyCode::Enter => return Ok(chosen),
                KeyCode::Esc => return Err("Selection cancelled".into()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("Selection cancelled".into());
                }
                _ => {}
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
        terminal.touch();
    }
}

fn draw_items(
    title: &str,
    items: &[TuiItem],
    selected: usize,
    chosen: &[usize],
    show_checkboxes: bool,
    message: Option<&str>,
    processing: Option<ProcessingIndicator>,
) -> Result<(), String> {
    let mut out = stdout();
    let (cols, rows) = size().unwrap_or((100, 30));
    let width = cols.max(4) as usize;
    let inner_width = width.saturating_sub(2);
    let list_height = rows.saturating_sub(7).max(1) as usize;
    let start = if selected >= list_height {
        selected + 1 - list_height
    } else {
        0
    };
    let visible_items = items.iter().enumerate().skip(start).take(list_height);
    let mut visible_count = 0usize;

    queue!(
        out,
        Clear(ClearType::All),
        MoveTo(0, 0),
        SetForegroundColor(Color::DarkGrey),
        Print(top_border(title, width)),
        ResetColor
    )
    .map_err(|err| err.to_string())?;

    let summary = if items.is_empty() {
        " No USB devices are currently exported. Waiting for hotplug...".to_string()
    } else {
        format!(
            " {} device(s) exported. Space toggles the selected port.",
            items.len()
        )
    };
    draw_content_line(&mut out, &summary, inner_width, Color::DarkGrey, None)?;
    draw_separator(&mut out, width)?;

    for (index, item) in visible_items {
        visible_count += 1;
        let marker = if selected == index { ">" } else { " " };
        let checked = if !show_checkboxes {
            ""
        } else if chosen.contains(&index) {
            "[x] "
        } else {
            "[ ] "
        };
        let is_processing = processing.is_some_and(|processing| processing.index == index);
        let row_color = if is_processing {
            Color::Yellow
        } else if item.label.starts_with("[x]") {
            Color::Green
        } else if selected == index {
            Color::White
        } else {
            Color::Grey
        };
        let row_bg = (selected == index).then_some(Color::DarkGrey);
        let label = if processing.is_some_and(|processing| processing.index == index) {
            label_with_spinner(&item.label, processing.unwrap().frame)
        } else {
            item.label.clone()
        };
        draw_content_line(
            &mut out,
            &format!(" {marker} {checked}{label}"),
            inner_width,
            row_color,
            row_bg,
        )?;
    }
    for _ in visible_count..list_height {
        draw_content_line(&mut out, "", inner_width, Color::Grey, None)?;
    }

    if let Some(message) = message {
        draw_separator(&mut out, width)?;
        let color = if message.starts_with("Error:") {
            Color::Red
        } else {
            Color::Yellow
        };
        draw_content_line(&mut out, &format!(" {message}"), inner_width, color, None)?;
    }

    draw_separator(&mut out, width)?;
    draw_status_bar(&mut out, inner_width)?;
    queue!(
        out,
        SetForegroundColor(Color::DarkGrey),
        Print(bottom_border(width)),
        ResetColor
    )
    .map_err(|err| err.to_string())?;

    out.flush().map_err(|err| err.to_string())
}

fn run_with_spinner<Action>(
    title: &str,
    items: &[TuiItem],
    selected: usize,
    action: Action,
) -> Result<String, String>
where
    Action: FnOnce() -> Result<String, String> + Send,
{
    thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        scope.spawn(move || {
            let _ = tx.send(action());
        });

        let mut frame = 0usize;
        loop {
            draw_items(
                title,
                items,
                selected,
                &[],
                false,
                None,
                Some(ProcessingIndicator {
                    index: selected,
                    frame,
                }),
            )?;
            frame = frame.wrapping_add(1);

            while event::poll(Duration::from_millis(0))
                .map_err(|err| format!("Failed to poll terminal event: {err}"))?
            {
                let event =
                    event::read().map_err(|err| format!("Failed to read terminal event: {err}"))?;
                if matches!(list_key_action(&event), Some(ListKeyAction::Cancel)) {
                    return Ok(
                        "Error: Operation is still running; usbip timeout will stop it shortly"
                            .to_string(),
                    );
                }
            }

            match rx.recv_timeout(Duration::from_millis(120)) {
                Ok(result) => {
                    return Ok(match result {
                        Ok(message) => message,
                        Err(err) => format!("Error: {err}"),
                    });
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Ok("Error: worker stopped unexpectedly".into());
                }
            }
        }
    })
}

fn top_border(title: &str, width: usize) -> String {
    let title = truncate_to_width(title, width.saturating_sub(6));
    let prefix = format!("┌ {title} ");
    let fill_width = width.saturating_sub(prefix.chars().count() + 1);
    format!("{prefix}{}┐\r\n", "─".repeat(fill_width))
}

fn bottom_border(width: usize) -> String {
    format!("└{}┘\r\n", "─".repeat(width.saturating_sub(2)))
}

fn draw_separator(out: &mut impl Write, width: usize) -> Result<(), String> {
    queue!(
        out,
        SetForegroundColor(Color::DarkGrey),
        Print(format!("├{}┤\r\n", "─".repeat(width.saturating_sub(2)))),
        ResetColor
    )
    .map_err(|err| err.to_string())
}

fn pad_to_width(value: &str, width: usize) -> String {
    let value = truncate_to_width(value, width);
    let padding = width.saturating_sub(value.chars().count());
    format!("{value}{}", " ".repeat(padding))
}

fn draw_content_line(
    out: &mut impl Write,
    text: &str,
    width: usize,
    foreground: Color,
    background: Option<Color>,
) -> Result<(), String> {
    queue!(
        out,
        SetForegroundColor(Color::DarkGrey),
        Print("│"),
        SetForegroundColor(foreground)
    )
    .map_err(|err| err.to_string())?;
    if let Some(background) = background {
        queue!(out, SetBackgroundColor(background)).map_err(|err| err.to_string())?;
    }
    queue!(
        out,
        Print(pad_to_width(text, width)),
        ResetColor,
        SetForegroundColor(Color::DarkGrey),
        Print("│\r\n"),
        ResetColor
    )
    .map_err(|err| err.to_string())
}

fn draw_status_bar(out: &mut impl Write, width: usize) -> Result<(), String> {
    let label_width = width.min(8);
    let controls_width = width.saturating_sub(label_width);
    queue!(
        out,
        SetForegroundColor(Color::DarkGrey),
        Print("│"),
        SetBackgroundColor(Color::Rgb {
            r: 99,
            g: 102,
            b: 241
        }),
        SetForegroundColor(Color::White),
        Print(pad_to_width(" LUSBIP", label_width)),
        SetBackgroundColor(Color::Rgb {
            r: 30,
            g: 30,
            b: 40
        }),
        SetForegroundColor(Color::White),
        Print(pad_to_width(
            "  ↑↓/j/k Move  Space Attach/Detach  Esc/Ctrl+C Detach & Quit",
            controls_width
        )),
        ResetColor,
        SetForegroundColor(Color::DarkGrey),
        Print("│\r\n"),
        ResetColor
    )
    .map_err(|err| err.to_string())
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self, String> {
        let mut out = stdout();
        execute!(out, EnterAlternateScreen, Hide).map_err(|err| err.to_string())?;
        enable_raw_mode().map_err(|err| {
            let _ = execute!(stdout(), LeaveAlternateScreen, Show);
            err.to_string()
        })?;
        drain_startup_events()?;
        Ok(Self)
    }

    fn touch(&mut self) {}
}

fn drain_startup_events() -> Result<(), String> {
    while event::poll(Duration::from_millis(0))
        .map_err(|err| format!("Failed to poll terminal event: {err}"))?
    {
        let event = event::read().map_err(|err| format!("Failed to read terminal event: {err}"))?;
        if !should_flush_startup_event(&event) {
            break;
        }
    }

    Ok(())
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, Show);
    }
}
