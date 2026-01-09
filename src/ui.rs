use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, StyledGrapheme},
    widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation},
};

use crate::{
    app::{App, Status, TerminalStatus},
    device_finder::Reactive,
};

fn render_input_block(input: &str, area: Rect, frame: &mut Frame) {
    let paragraph = Paragraph::new(input)
        .block(Block::bordered())
        .left_aligned();
    frame.render_widget(paragraph, area);
}

fn render_log<'a, T: Iterator>(lines: T, area: Rect, frame: &mut Frame)
where
    std::borrow::Cow<'a, str>: std::convert::From<<T as std::iter::Iterator>::Item>,
    <T as std::iter::Iterator>::Item: std::fmt::Display,
{
    let lines = lines.enumerate().take((area.height).into());
    let buf = frame.buffer_mut();
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
            let x = (x as u16).saturating_add(area.x);
            let y = area.height + area.y - (lineno as u16 + 1);
            let Some(cell) = buf.cell_mut((x, y)) else {
                break;
            };
            cell.set_style(s);
            cell.set_symbol(g);
        }
    }
}

fn render_status_block(stat: &Status, area: Rect, frame: &mut Frame) {
    let [stats, log_area] =
        &*Layout::vertical([Constraint::Percentage(30), Constraint::Percentage(70)]).split(area)
    else {
        panic!("Status constraint failed!");
    };

    const ON: &str = "●";
    const OFF: &str = "○";
    let cts = if stat.cts { ON } else { OFF };
    let dtr = if stat.dtr { ON } else { OFF };

    let log_block = Block::bordered();
    let log_zone = log_block.inner(*log_area);
    frame.render_widget(log_block, *log_area);
    let lines = stat.log.iter().rev();
    render_log(lines, log_zone, frame);

    let status = format!("CTS: {}\nDTR: {}\nConnected: {}", cts, dtr, stat.device,);

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
    let lines = input.text.iter().rev().skip(input.scroll_index);
    render_log(lines, text_area, frame);
}

fn render_popup(popup: &mut dyn Reactive, area: Rect, frame: &mut Frame) {
    let x_margin = area.width / 4;
    let y_margin = area.height / 4;
    let area = area.inner(ratatui::layout::Margin {
        horizontal: x_margin,
        vertical: y_margin,
    });

    let buf = frame.buffer_mut();
    popup.draw(area, buf);
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

    render_terminal_block(&mut state.term_state, *term, frame);
    render_input_block(&state.term_input, *input, frame);
    render_status_block(&state.status, *status_area, frame);
    if let Some(popup) = state.popup.as_mut() {
        render_popup(popup.as_mut(), area, frame);
    }
}
