use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, StyledGrapheme},
    widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation, StatefulWidget, Widget},
    Frame,
};

use crate::app::{App, Status, TerminalStatus};

fn render_input_block(input: &str, area: Rect, buf: &mut Buffer) {
    let paragraph = Paragraph::new(input)
        .block(Block::bordered())
        .left_aligned();
    paragraph.render(area, buf);
}

fn render_status_block(stat: &Status, area: Rect, buf: &mut Buffer) {
    const ON: &str = "●";
    const OFF: &str = "○";
    let cts = if stat.cts { ON } else { OFF };
    let dtr = if stat.dtr { ON } else { OFF };

    let status = format!("CTS: {}\nDTR: {}\n", cts, dtr);

    let status_block = Paragraph::new(status).block(Block::bordered()).centered();
    status_block.render(area, buf);
}

fn render_terminal_block(input: &mut TerminalStatus, area: Rect, buf: &mut Buffer) {
    let [text_area, scrollbar_area] =
        &*Layout::horizontal([Constraint::Min(20), Constraint::Length(1)]).split(area)
    else {
        panic!("Terminal constraint failed");
    };
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
    input.scroll_state = input
        .scroll_state
        .content_length(input.text.len())
        .position(input.text.len() - input.scroll_index);

    scrollbar.render(*scrollbar_area, buf, &mut input.scroll_state);
    let block = Block::bordered();
    let text_area = block.inner(*text_area);
    block.render(area, buf);
    let lines = input
        .text
        .iter()
        .rev()
        .skip(input.scroll_index)
        .enumerate()
        .take(text_area.height.into());
    for (lineno, line) in lines {
        let y = input.text.len() - lineno;
        let line = Line::raw(line);
        for (
            x,
            StyledGrapheme {
                style: s,
                symbol: g,
            },
        ) in line.styled_graphemes(Style::default()).enumerate()
        {
            let Some(cell) = buf.cell_mut((x as u16, y as u16)) else {
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
    let buf = frame.buffer_mut();
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

    render_input_block(&state.term_input, *input, buf);
    render_status_block(&state.status, *status_area, buf);
    render_terminal_block(&mut state.term_state, *term, buf);
}
