use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, StyledGrapheme},
    widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation, StatefulWidget, Widget},
    Frame,
};

use crate::app::{App, Status, TerminalStatus};

fn render_input_block(input: &str, area: Rect, frame: &mut Frame) {
    let paragraph = Paragraph::new(input)
        .block(Block::bordered())
        .left_aligned();
    frame.render_widget(paragraph, area);
}

fn render_status_block(stat: &Status, area: Rect, frame: &mut Frame) {
    let [stats, log] =
        &*Layout::vertical([Constraint::Percentage(30), Constraint::Percentage(70)]).split(area)
    else {
        panic!("Status constraint failed!");
    };

    const ON: &str = "●";
    const OFF: &str = "○";
    let cts = if stat.cts { ON } else { OFF };
    let dtr = if stat.dtr { ON } else { OFF };

    let status = format!("CTS: {}\nDTR: {}\nConnected: {}", cts, dtr, stat.device);

    let status_block = Paragraph::new(status).block(Block::bordered()).centered();
    frame.render_widget(status_block, *stats);
}

fn render_terminal_block(input: &mut TerminalStatus, area: Rect, frame: &mut Frame) {
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_symbol("#");
    input.scroll_state = input
        .scroll_state
        .content_length(input.text.len())
        .position(input.text.len() - input.scroll_index);
    frame.render_stateful_widget(scrollbar, area, &mut input.scroll_state);
    let block = Block::bordered();
    let text_area = block.inner(area);
    frame.render_widget(block, area);
    let buf = frame.buffer_mut();
    let lines = input
        .text
        .iter()
        .rev()
        .skip(input.scroll_index)
        .enumerate()
        .take((text_area.height + 1).into());
    for (lineno, line) in lines {
        let line = Line::raw(line);
        for (
            x,
            StyledGrapheme {
                style: s,
                symbol: g,
            },
        ) in line.styled_graphemes(Style::default()).enumerate()
        {
            let x = (x as u16).saturating_add(text_area.x);
            let y = text_area.height + text_area.y - (lineno as u16);
            let Some(cell) = buf.cell_mut((x, y)) else {
                break;
            };
            cell.set_style(s);
            cell.set_symbol(g);
        }
    }
}

pub fn render_ui(state: &mut App, frame: &mut Frame) {
    use ratatui::layout::Direction;
    let area = frame.area();
    let a = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Min(20)])
        .split(area);
    let [bigger, status_area] = &*a else {
        panic!("Bigger area should have 1 item");
    };

    let left_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(90), Constraint::Min(1)])
        .split(*bigger);

    let [term, input] = &*left_area else {
        // really should not happen unless above fails somehow
        panic!("Layout should have 2 items only");
    };

    render_input_block(&state.term_input, *input, frame);
    render_status_block(&state.status, *status_area, frame);
    render_terminal_block(&mut state.term_state, *term, frame);
}
