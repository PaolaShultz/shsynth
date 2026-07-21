use crate::performance_meter::{self, BarCell, MeterColor};
use ratatui::{
    backend::Backend,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Clear, Paragraph},
    Frame,
};
use std::time::Duration;

pub const MINIMUM_VISIBLE: Duration = Duration::from_millis(900);
pub const INPUT_RESCAN_INTERVAL: Duration = Duration::from_millis(500);

pub const fn qualified_input_available(terminal_keyboard: bool, midi_input: bool) -> bool {
    terminal_keyboard || midi_input
}

pub fn waiting_for_input(elapsed: Duration, terminal_keyboard: bool, midi_input: bool) -> bool {
    elapsed < MINIMUM_VISIBLE || !qualified_input_available(terminal_keyboard, midi_input)
}

fn meter_color(color: MeterColor) -> Color {
    match color {
        MeterColor::Green => Color::Green,
        MeterColor::Yellow => Color::LightYellow,
        MeterColor::Red => Color::Red,
    }
}

fn styled_bar(cells: &[BarCell]) -> Vec<Span<'static>> {
    cells
        .iter()
        .map(|cell| {
            Span::styled(
                cell.symbol.to_string(),
                Style::default()
                    .fg(meter_color(cell.color))
                    .add_modifier(if cell.symbol == '·' {
                        Modifier::empty()
                    } else {
                        Modifier::BOLD
                    }),
            )
        })
        .collect()
}

fn animated_level(elapsed: Duration, channel: usize) -> (f32, f32) {
    // Fixed envelopes keep the animation deterministic while giving the two
    // channels different, musical-looking attack and release movement.
    const LEFT: [f32; 16] = [
        -36.0, -31.0, -22.0, -15.0, -8.5, -5.0, -11.0, -18.0, -13.0, -7.0, -2.4, -9.0, -17.0,
        -24.0, -19.0, -12.0,
    ];
    const RIGHT: [f32; 16] = [
        -30.0, -24.0, -18.0, -11.0, -6.5, -13.0, -20.0, -15.0, -9.0, -4.0, -8.0, -14.0, -10.0,
        -3.0, -12.0, -21.0,
    ];
    let frame = (elapsed.as_millis() / 85) as usize;
    let rms = if channel == 0 {
        LEFT[frame % LEFT.len()]
    } else {
        RIGHT[frame % RIGHT.len()]
    };
    (rms, (rms + 6.0).min(-0.2))
}

fn thick_meter(label: char, width: u16, elapsed: Duration, channel: usize) -> Vec<Spans<'static>> {
    let bar_width = usize::from(width.saturating_sub(4)).max(1);
    let (rms, peak) = animated_level(elapsed, channel);
    let cells = performance_meter::audio_bar(bar_width, rms, peak);
    (0..3)
        .map(|row| {
            let mut spans = vec![Span::styled(
                if row == 1 {
                    format!("{label} [")
                } else {
                    "  [".into()
                },
                Style::default().fg(Color::White).add_modifier(if row == 1 {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            )];
            spans.extend(styled_bar(&cells));
            spans.push(Span::styled("]", Style::default().fg(Color::White)));
            Spans::from(spans)
        })
        .collect()
}

fn meter_area(area: Rect, y: u16) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(y),
        area.width.saturating_sub(2),
        3,
    )
}

pub fn draw<B: Backend>(
    frame: &mut Frame<B>,
    elapsed: Duration,
    input_available: bool,
    expected_midi: Option<&str>,
    build_badge: &str,
) {
    let area = frame.size();
    frame.render_widget(Clear, area);

    let compact = area.height < 14;
    let title_y = if compact { 0 } else { 1 };
    frame.render_widget(
        Paragraph::new(format!("{build_badge} · SHR-DAW"))
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        Rect::new(area.x, area.y.saturating_add(title_y), area.width, 1),
    );

    if !compact {
        frame.render_widget(
            Paragraph::new("STEREO VU · STARTUP")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray)),
            Rect::new(area.x, area.y.saturating_add(3), area.width, 1),
        );
    }

    let left_y: u16 = if compact { 1 } else { 5 };
    let right_y = left_y.saturating_add(4);
    let meter_width = area.width.saturating_sub(2);
    frame.render_widget(
        Paragraph::new(thick_meter('L', meter_width, elapsed, 0)),
        meter_area(area, left_y),
    );
    frame.render_widget(
        Paragraph::new(thick_meter('R', meter_width, elapsed, 1)),
        meter_area(area, right_y),
    );

    let status = if input_available {
        "LOADING"
    } else {
        "CONNECT KEYBOARD OR MIDI INPUT"
    };
    let status_y = if compact {
        area.height.saturating_sub(1)
    } else {
        right_y.saturating_add(5)
    };
    frame.render_widget(
        Paragraph::new(status).alignment(Alignment::Center).style(
            Style::default()
                .fg(if input_available {
                    Color::Gray
                } else {
                    Color::LightYellow
                })
                .add_modifier(if input_available {
                    Modifier::empty()
                } else {
                    Modifier::BOLD
                }),
        ),
        Rect::new(area.x, area.y.saturating_add(status_y), area.width, 1),
    );

    if !compact && !input_available {
        if let Some(expected) = expected_midi.filter(|name| !name.trim().is_empty()) {
            frame.render_widget(
                Paragraph::new(format!("WAITING FOR {expected}"))
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                Rect::new(
                    area.x.saturating_add(1),
                    area.y.saturating_add(status_y.saturating_add(2)),
                    area.width.saturating_sub(2),
                    1,
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

    fn render(input_available: bool) -> Buffer {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                draw(
                    frame,
                    Duration::from_millis(510),
                    input_available,
                    Some("Stage Keyboard"),
                    "DEV",
                )
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn text(buffer: &Buffer) -> String {
        buffer
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect()
    }

    #[test]
    fn keyboard_and_midi_are_equal_qualified_inputs() {
        assert!(qualified_input_available(true, false));
        assert!(qualified_input_available(false, true));
        assert!(qualified_input_available(true, true));
        assert!(!qualified_input_available(false, false));
    }

    #[test]
    fn splash_observes_minimum_time_but_only_waits_without_any_input() {
        assert!(waiting_for_input(Duration::ZERO, true, false));
        assert!(!waiting_for_input(MINIMUM_VISIBLE, true, false));
        assert!(!waiting_for_input(MINIMUM_VISIBLE, false, true));
        assert!(waiting_for_input(MINIMUM_VISIBLE, false, false));
    }

    #[test]
    fn splash_renders_thick_stereo_meters_with_real_vu_colours() {
        let buffer = render(true);
        let output = text(&buffer);
        assert!(output.contains("DEV · SHR-DAW"));
        assert!(output.contains("STEREO VU · STARTUP"));
        assert!(output.contains("L ["));
        assert!(output.contains("R ["));
        assert!(output.contains("LOADING"));

        for rows in [5..8, 9..12] {
            for y in rows {
                let coloured = (0..40).map(|x| buffer.get(x, y).fg).collect::<Vec<Color>>();
                assert!(coloured.contains(&Color::Green));
                assert!(coloured.contains(&Color::LightYellow));
                assert!(coloured.contains(&Color::Red));
            }
        }
    }

    #[test]
    fn splash_names_missing_input_only_in_waiting_state() {
        let waiting = text(&render(false));
        assert!(waiting.contains("CONNECT KEYBOARD OR MIDI INPUT"));
        assert!(waiting.contains("WAITING FOR Stage Keyboard"));

        let loading = text(&render(true));
        assert!(!loading.contains("WAITING FOR"));
    }
}
