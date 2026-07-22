use crate::performance_meter::{self, BarCell, LedState, MeterColor};
use ratatui::{
    backend::Backend,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Clear, Paragraph},
    Frame,
};
use std::time::Duration;

pub const MINIMUM_VISIBLE: Duration = Duration::from_secs(2);
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
            let active = cell.state != LedState::Off;
            Span::styled(
                "●",
                Style::default()
                    .fg(if active {
                        meter_color(cell.color)
                    } else {
                        Color::DarkGray
                    })
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            )
        })
        .collect()
}

fn animated_level(elapsed: Duration) -> f32 {
    // One deterministic envelope keeps the decorative channels coherent. The
    // splash previews the LED language; it does not pretend to meter audio.
    const LEVELS: [f32; 16] = [
        -36.0, -31.0, -22.0, -15.0, -8.5, -5.0, -11.0, -18.0, -13.0, -7.0, -2.4, -9.0, -17.0,
        -24.0, -19.0, -12.0,
    ];
    let frame = (elapsed.as_millis() / 85) as usize;
    LEVELS[frame % LEVELS.len()]
}

fn thick_meter(label: char, width: u16, elapsed: Duration) -> Vec<Spans<'static>> {
    let bar_width = usize::from(width.saturating_sub(4)).max(1);
    let rms = animated_level(elapsed);
    let cells = performance_meter::audio_bar(bar_width, rms, performance_meter::AUDIO_FLOOR_DBFS);
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
        Paragraph::new(thick_meter('L', meter_width, elapsed)),
        meter_area(area, left_y),
    );
    frame.render_widget(
        Paragraph::new(thick_meter('R', meter_width, elapsed)),
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
        let backend = TestBackend::new(40, 13);
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
    fn splash_renders_three_circular_led_rows_per_channel_at_40x13() {
        let buffer = render(true);
        let output = text(&buffer);
        assert!(output.contains("DEV · SHR-DAW"));
        assert!(output.contains("L ["));
        assert!(output.contains("R ["));
        assert!(output.contains("LOADING"));

        for rows in [1..4, 5..8] {
            for y in rows {
                let symbols = (0..40)
                    .map(|x| buffer.get(x, y).symbol.as_str())
                    .collect::<String>();
                assert!(symbols.contains('●'));
                assert!(!symbols.contains('█'));
                assert!(!symbols.contains('│'));
            }
        }
        for x in 4..38 {
            assert_eq!(buffer.get(x, 1).symbol, buffer.get(x, 5).symbol);
            assert_eq!(buffer.get(x, 1).fg, buffer.get(x, 5).fg);
        }
    }

    #[test]
    fn splash_names_missing_input_only_in_waiting_state() {
        let waiting = text(&render(false));
        assert!(waiting.contains("CONNECT KEYBOARD OR MIDI INPUT"));

        let loading = text(&render(true));
        assert!(!loading.contains("WAITING FOR"));
    }
}
