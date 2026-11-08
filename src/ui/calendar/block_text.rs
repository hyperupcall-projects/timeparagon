//! Fitting text into event blocks on the canvas.

use super::*;

/// Use the GPU glyph atlas for readable, hinted calendar labels. Software
/// builds retain cached outlines to preserve clipping while scrolling.
pub(super) fn fill_glyphs(frame: &mut Frame, text: impl Into<Text>) {
    #[cfg(feature = "wgpu")]
    frame.fill_text(text);
    #[cfg(not(feature = "wgpu"))]
    text.into()
        .draw_with(|glyph, color| frame.fill(&glyph, color));
}

/// Draws a block's title and the time under it, inside the block.
///
/// A Google summary is free text: it can be longer than the event it names,
/// it can bring its own line breaks, and it can hold a word too wide for the
/// block on its own. The title is broken into the lines that fit and cut
/// short with an ellipsis when there are more of them than the block has
/// room for, so a label never reaches past the edge of the event it belongs
/// to and lands on top of the ones below it.
pub(super) fn draw_block_label(
    frame: &mut Frame,
    bounds: Rectangle,
    title: &str,
    when: &str,
    ink: Color,
    title_size: f32,
) {
    let when_size = title_size - 1.0;
    let title_line = line_height(title_size);
    let when_line = line_height(when_size);
    let text_width = (bounds.width - BLOCK_PADDING - BLOCK_PADDING_RIGHT).max(8.0);

    let (top, room) = text_room(bounds.height, title_size);
    let title_top = bounds.y + top;

    let lines = fit_lines(title, title_size, text_width, room);

    for (index, line) in lines.iter().enumerate() {
        fill_glyphs(
            frame,
            Text {
                content: line.clone(),
                position: Point::new(
                    bounds.x + BLOCK_PADDING,
                    title_top + index as f32 * title_line,
                ),
                color: ink,
                size: Pixels(title_size),
                font: Font::DEFAULT,
                align_x: iced::alignment::Horizontal::Left.into(),
                align_y: iced::alignment::Vertical::Top,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Basic,
                max_width: text_width,
            },
        );
    }

    // The time sits under the title, so it has to clear however many lines
    // the title took rather than a fixed one-line offset.
    let when_top = title_top + lines.len() as f32 * title_line + BLOCK_GAP;
    for (index, line) in when.lines().enumerate() {
        let top = when_top + index as f32 * when_line;
        if top + when_line > bounds.y + bounds.height {
            break;
        }
        let Some(when) = time_label(line, when_size, text_width) else {
            continue;
        };
        fill_glyphs(
            frame,
            Text {
                content: when,
                position: Point::new(bounds.x + BLOCK_PADDING, top),
                color: ink.scale_alpha(0.85),
                size: Pixels(when_size),
                font: Font::DEFAULT,
                align_x: iced::alignment::Horizontal::Left.into(),
                align_y: iced::alignment::Vertical::Top,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Basic,
                max_width: text_width,
            },
        );
    }
}

/// The time to write under a title, if there is one that fits.
///
/// A block too narrow for the whole range keeps its start time, which is the
/// half that says when the event is.
pub(super) fn time_label(when: &str, size: f32, width: f32) -> Option<String> {
    if text_width_of(when, size) <= width {
        return Some(when.to_owned());
    }

    when.split_once(" – ")
        .filter(|(start, _)| text_width_of(start, size) <= width)
        .map(|(start, _)| start.to_owned())
}

/// The gap above a block's first line of text, and how many lines fit below it.
///
/// The gap shrinks on a short block, so that a line of text still fits inside
/// it rather than hanging over the edge.
pub(super) fn text_room(height: f32, size: f32) -> (f32, usize) {
    let line = line_height(size);
    let top = (height - line).clamp(0.0, BLOCK_TOP);

    (top, ((height - top) / line).floor().max(0.0) as usize)
}

/// The lines of a title that fit in `room` lines of `width`.
///
/// A title that needs more lines than the block has room for is cut short and
/// marked with an ellipsis, so that it stays inside the block.
pub(super) fn fit_lines(title: &str, size: f32, width: f32, room: usize) -> Vec<String> {
    if room == 0 {
        return Vec::new();
    }

    let mut lines = wrap_title(title, size, width);

    if lines.len() > room {
        lines.truncate(room);

        if let Some(last) = lines.last_mut() {
            *last = cut_line(last, size, width);
        }
    }

    lines
}

/// Breaks a title into the lines it takes inside a block `width` wide.
///
/// Words are broken at the character level when they are too wide to sit on a
/// line of their own, so that no line reaches past the edge of the block.
pub(super) fn wrap_title(title: &str, size: f32, width: f32) -> Vec<String> {
    // A title that fits on one line is the common case, and it costs a single
    // measurement.
    if !title.contains('\n') && text_width_of(title, size) <= width {
        return vec![title.to_owned()];
    }

    let mut lines = Vec::new();

    for source in title.split('\n') {
        let mut line = String::new();

        for word in source.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };

            if text_width_of(&candidate, size) <= width {
                line = candidate;
                continue;
            }

            // The word does not fit on the line being built, so that line is
            // finished off and the word goes on one of its own.
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }

            if text_width_of(word, size) <= width {
                line = word.to_owned();
                continue;
            }

            // The word is too wide for a line of its own as well, so it is
            // broken up at the character level rather than left to reach past
            // the edge of the block.
            let pieces = break_word(word, size, width);
            let last = pieces.len().saturating_sub(1);

            for (index, piece) in pieces.into_iter().enumerate() {
                if index == last {
                    line = piece;
                } else {
                    lines.push(piece);
                }
            }
        }

        lines.push(line);
    }

    lines
}

/// Breaks a word too wide for a line into the pieces that fit.
pub(super) fn break_word(word: &str, size: f32, width: f32) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();

    for character in word.chars() {
        let candidate = format!("{piece}{character}");

        if !piece.is_empty() && text_width_of(&candidate, size) > width {
            pieces.push(std::mem::take(&mut piece));
        }

        piece.push(character);
    }

    if !piece.is_empty() {
        pieces.push(piece);
    }

    pieces
}

/// A line with an ellipsis on the end, cut short enough to fit `width`.
pub(super) fn cut_line(line: &str, size: f32, width: f32) -> String {
    let characters: Vec<char> = line.chars().collect();

    // The longest run of the line that still leaves room for the ellipsis.
    let (mut low, mut high) = (0usize, characters.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate: String = characters[..middle].iter().collect();

        if text_width_of(&format!("{candidate}{ELLIPSIS}"), size) <= width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    let prefix: String = characters[..low].iter().collect();
    format!("{prefix}{ELLIPSIS}")
}

/// A line as it fits `width`, cut short with an ellipsis when it does not.
pub(super) fn fit_line(line: &str, size: f32, width: f32) -> String {
    if text_width_of(line, size) <= width {
        line.to_owned()
    } else {
        cut_line(line, size, width)
    }
}

/// The height of one line of text at `size`.
pub(super) fn line_height(size: f32) -> f32 {
    text::LineHeight::default().to_absolute(Pixels(size)).0
}

/// The width a string takes on a single line.
pub(super) fn text_width_of(content: &str, size: f32) -> f32 {
    use iced::advanced::text::Paragraph as _;

    paragraph(content, size, MEASURE_WIDTH).min_width()
}

/// A paragraph laid out for measuring.
///
/// The width is wide enough that nothing wraps, so the paragraph measures the
/// text as it is written rather than as it would be broken up.
pub(super) fn paragraph(
    content: &str,
    size: f32,
    width: f32,
) -> iced::advanced::graphics::text::Paragraph {
    use iced::advanced::graphics::text::Paragraph;
    use iced::advanced::text::Paragraph as _;

    Paragraph::with_text(iced::advanced::text::Text {
        content,
        bounds: Size::new(width, f32::INFINITY),
        size: Pixels(size),
        line_height: text::LineHeight::default(),
        font: Font::DEFAULT,
        align_x: iced::advanced::text::Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping: text::Shaping::Basic,
        wrapping: text::Wrapping::None,
    })
}

/// Everything the cached geometry is drawn from, so a rebuilt [`Block`] with a
/// different label or shade retraces instead of going stale.
pub(super) fn cache_key(
    blocks: &[Block],
    hour_height: f32,
    gutter: f32,
    compact: bool,
    day_start_hour: u32,
    dragging: Option<usize>,
) -> u64 {
    let mut hash = hour_height.to_bits() as u64;
    hash = hash.wrapping_mul(31).wrapping_add(gutter.to_bits() as u64);
    hash = hash.wrapping_mul(31).wrapping_add(u64::from(compact));
    hash = hash
        .wrapping_mul(31)
        .wrapping_add(u64::from(day_start_hour));
    // Which block is being dragged decides which one the cached grid leaves
    // out, so the grid has to retrace when a drag starts or ends.
    hash = hash
        .wrapping_mul(31)
        .wrapping_add(dragging.map_or(u64::MAX, |index| index as u64));

    for block in blocks {
        hash = hash.wrapping_mul(31).wrapping_add(block.start_min as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.end_min as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.column as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.columns as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.column_span as u64);
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(block.color.r.to_bits() as u64);
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(block.color.g.to_bits() as u64);
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(block.color.b.to_bits() as u64);
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(block.color.a.to_bits() as u64);
        // An unconfirmed write changes the shade, so it has to retrace too.
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(block.pending));
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(u64::from(block.time_block));
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(u64::from(block.habit.is_some()))
            .wrapping_mul(31)
            .wrapping_add(match block.habit_completed {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            });
        for (id, title, minutes) in &block.tasks {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(*minutes));
            for byte in id.bytes().chain(title.bytes()) {
                hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
            }
        }
        for byte in block.title.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        for byte in block.when.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
    }

    hash
}

/// The part of the grid's cache key that comes from the travel boxes.
pub(super) fn travel_key(travel: &[TravelBox]) -> u64 {
    let mut hash = 0u64;

    for trip in travel {
        hash = hash.wrapping_mul(31).wrapping_add(trip.anchor_min as u64);
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(trip.below));
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(trip.minutes));
        hash = hash.wrapping_mul(31).wrapping_add(trip.column as u64);
        hash = hash.wrapping_mul(31).wrapping_add(trip.columns as u64);
        hash = hash.wrapping_mul(31).wrapping_add(trip.column_span as u64);
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(u64::from(trip.time_block));
        for byte in trip.label.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
    }

    hash
}

/// How far one notch of the wheel zooms.
pub(super) const WHEEL_ZOOM: f32 = 1.12;

/// Wheel notches to apply. Scroll up is positive.
///
/// Pixel deltas from a trackpad are large, so they are scaled down to the
/// same step as a mouse notch. Whichever axis moved more wins, because some
/// mice and desktops report the wheel on the other axis.
pub(super) fn zoom_steps(delta: mouse::ScrollDelta) -> f32 {
    let (x, y, scale) = match delta {
        mouse::ScrollDelta::Lines { x, y } => (x, y, 1.0),
        mouse::ScrollDelta::Pixels { x, y } => (x, y, 1.0 / 60.0),
    };
    let dominant = if x.abs() > y.abs() { x } else { y };
    dominant * scale
}
