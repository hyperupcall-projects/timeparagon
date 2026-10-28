//! Software rendering for the dock: an opaque strip down one edge, and a hover
//! card drawn beside it into the same transparent surface.
//!
//! Everything is laid out in logical pixels and multiplied by the output scale
//! only when it touches the pixmap, so the pointer maths never needs to know
//! about HiDPI.
//!
//! The pixmap is painted in blue-green-red byte order, which is what a
//! `wl_shm` ARGB8888 buffer holds on little-endian machines, so a frame goes
//! to the compositor as a plain copy. tiny-skia treats the channels alike, so
//! the only places that care are where a colour enters: [`paint_for`] and
//! [`text_color`] swap red and blue on the way in.

use std::sync::mpsc;

use cosmic_text::{
    Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight,
};
use iced::Color;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use super::config::DockConfig;
use super::model::{Card, DAY_MINUTES, Segment};
use crate::config::DockSide;

/// Shortest a segment is ever drawn, so a five-minute task is still hoverable.
const MIN_SEGMENT_HEIGHT: f32 = 3.0;
/// Space between the back button and the start of the day.
const BUTTON_GAP: f32 = 2.0;
/// Thickness of the red "now" line.
const NOW_HEIGHT: f32 = 2.0;
/// Past events fade to this opacity.
const PAST_ALPHA: f32 = 0.5;
const CARD_PADDING: f32 = 10.0;
const CARD_RADIUS: f32 = 8.0;
const TITLE_SIZE: f32 = 14.0;
const BODY_SIZE: f32 = 12.0;
const LINE_SPACING: f32 = 1.3;
const BLOCK_GAP: f32 = 4.0;
/// Hour lines are a faint gray.
const HOUR_GRAY: Color = Color {
    r: 0.6,
    g: 0.6,
    b: 0.6,
    a: 0.55,
};
const HOUR_LINE_HEIGHT: f32 = 1.0;

/// The theme colors the dock paints with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// The day view's background, which also fills the gaps in the strip.
    pub background: Color,
    pub card: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub now: Color,
    pub accent: Color,
}

impl Palette {
    pub fn from_theme(theme: &iced::Theme) -> Self {
        let palette = theme.extended_palette();
        let text = palette.background.base.text;

        Self {
            background: palette.background.base.color,
            card: palette.background.weak.color,
            border: palette.background.strong.color,
            text,
            muted: Color { a: 0.7, ..text },
            now: palette.danger.base.color,
            accent: palette.primary.base.color,
        }
    }
}

/// Where everything sits on the surface, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub side: DockSide,
    /// The strip's outer rectangle, including its borders.
    pub strip_x: f32,
    pub strip_y: f32,
    pub strip_width: f32,
    pub strip_height: f32,
    /// The part of the strip inside the borders, where events are drawn.
    pub content_x: f32,
    pub content_y: f32,
    pub content_width: f32,
    pub content_height: f32,
    /// Thickness of the strip across its short axis.
    pub thickness: f32,
    /// Space between the strip and the hover card.
    pub gap: f32,
    /// Size of the hover card along its fixed axis: its width beside a
    /// vertical edge, its height beside a horizontal one.
    pub card_size: f32,
    pub border_outer: f32,
    pub border_inner: f32,
    pub border_outer_color: Color,
    pub border_inner_color: Color,
    pub scale: f32,
}

impl Layout {
    pub fn new(config: &DockConfig, width: u32, height: u32, scale: u32) -> Self {
        let width = width as f32;
        let height = height as f32;
        let thickness = config.width as f32;
        let gap = config.popup_gap as f32;

        let (strip_x, strip_y, strip_width, strip_height) = match config.side {
            DockSide::Left => (0.0, 0.0, thickness, height),
            DockSide::Right => (width - thickness, 0.0, thickness, height),
            DockSide::Top => (0.0, 0.0, width, thickness),
            DockSide::Bottom => (0.0, height - thickness, width, thickness),
        };

        // Keep at least one pixel of event area, however thick the borders
        // are for the chosen strip width.
        let inset = config.border_inset();
        let border_outer = config.border_outer_width.min(inset);
        let border_inner = config
            .border_inner_width
            .min((inset - border_outer).max(0.0));

        let content_x = strip_x + inset;
        let content_y = strip_y + inset;
        let content_width = (strip_width - 2.0 * inset).max(0.0);
        let content_height = (strip_height - 2.0 * inset).max(0.0);

        Self {
            width,
            height,
            side: config.side,
            strip_x,
            strip_y,
            strip_width,
            strip_height,
            content_x,
            content_y,
            content_width,
            content_height,
            thickness,
            gap,
            card_size: config.popup_width as f32,
            border_outer,
            border_inner,
            border_outer_color: config.border_outer_color.0,
            border_inner_color: config.border_inner_color.0,
            scale: scale.max(1) as f32,
        }
    }

    fn vertical(&self) -> bool {
        self.side.is_vertical()
    }

    /// The square back button's side, which is the usable strip thickness.
    fn button_size(&self) -> f32 {
        if self.vertical() {
            self.content_width
        } else {
            self.content_height
        }
    }

    /// Length of the band at the start of the day that holds the back button.
    fn header(&self) -> f32 {
        self.button_size() + BUTTON_GAP
    }

    /// The length of the strip the day is laid out across.
    fn day_length(&self) -> f32 {
        if self.vertical() {
            self.content_height
        } else {
            self.content_width
        }
    }

    fn axis_start(&self) -> f32 {
        if self.vertical() {
            self.content_y
        } else {
            self.content_x
        }
    }

    /// The distance along the strip at which a minute is drawn.
    fn along_of(&self, minute: f32) -> f32 {
        let start = self.axis_start();
        let header = self.header();
        start + header + minute / DAY_MINUTES as f32 * (self.day_length() - header).max(1.0)
    }

    /// The back button: a square at the start of the strip.
    pub fn button_rect(&self) -> (f32, f32, f32, f32) {
        let size = self.button_size();
        if self.vertical() {
            (self.content_x, self.content_y, self.content_width, size)
        } else {
            (self.content_x, self.content_y, size, self.content_height)
        }
    }

    pub fn hit_button(&self, x: f64, y: f64) -> bool {
        let (left, top, width, height) = self.button_rect();
        let (x, y) = (x as f32, y as f32);
        x >= left && x < left + width && y >= top && y < top + height
    }

    /// The logical rectangle (x, y, width, height) of a segment.
    pub fn segment_rect(&self, segment: &Segment) -> (f32, f32, f32, f32) {
        let start = self.along_of(segment.start_min as f32);
        let end = self.along_of(segment.end_min as f32);
        let length = (end - start).max(MIN_SEGMENT_HEIGHT);
        if self.vertical() {
            (self.content_x, start, self.content_width, length)
        } else {
            (start, self.content_y, length, self.content_height)
        }
    }

    /// The logical strip rectangle, which is also the surface's input region.
    pub fn strip_rect(&self) -> (f32, f32, f32, f32) {
        (
            self.strip_x,
            self.strip_y,
            self.strip_width,
            self.strip_height,
        )
    }

    /// Where the hover card sits when it describes something at `anchor`
    /// along the strip. `height` is the card's wanted height; it is clamped to
    /// the room the surface has for it.
    pub fn card_rect(&self, anchor: f32, height: f32) -> (f32, f32, f32, f32) {
        let width = self.card_size.min(self.width).max(0.0);
        let height = height.min(self.card_room()).max(0.0);

        if self.vertical() {
            let x = match self.side {
                DockSide::Left => self.strip_x + self.strip_width + self.gap,
                _ => self.strip_x - self.gap - width,
            };
            let y = anchor.min(self.height - height).max(0.0);
            (x, y, width, height)
        } else {
            let x = anchor.min(self.width - width).max(0.0);
            let y = match self.side {
                DockSide::Top => self.strip_y + self.strip_height + self.gap,
                _ => self.strip_y - self.gap - height,
            };
            let y = y.min(self.height - height).max(0.0);
            (x, y, width, height)
        }
    }

    /// How tall a hover card may be on this surface.
    fn card_room(&self) -> f32 {
        if self.vertical() {
            self.height
        } else if self.side == DockSide::Top {
            (self.height - self.strip_height - self.gap).max(0.0)
        } else {
            (self.strip_y - self.gap).max(0.0)
        }
    }

    /// The segment under a pointer position, if any.
    pub fn hit(&self, segments: &[Segment], x: f64, y: f64) -> Option<usize> {
        let (x, y) = (x as f32, y as f32);
        if x < self.content_x
            || x >= self.content_x + self.content_width
            || y < self.content_y
            || y >= self.content_y + self.content_height
        {
            return None;
        }

        let along = if self.vertical() {
            y - self.content_y
        } else {
            x - self.content_x
        };
        if along < self.header() {
            return None;
        }

        segments.iter().position(|segment| {
            let (left, top, width, height) = self.segment_rect(segment);
            x >= left && x < left + width && y >= top && y < top + height
        })
    }

    fn physical(&self, value: f32) -> f32 {
        value * self.scale
    }

    pub fn pixel_size(&self) -> (u32, u32) {
        (
            self.physical(self.width).round().max(1.0) as u32,
            self.physical(self.height).round().max(1.0) as u32,
        )
    }
}

/// What to paint this frame.
pub struct Scene<'a> {
    pub segments: &'a [Segment],
    pub now_minute: Option<f32>,
    pub hovered: Option<usize>,
    /// The segment whose full card was left open by a click.
    pub pinned: Option<usize>,
    /// Minutes past midnight at which the strip's day begins, which places
    /// the hour lines.
    pub day_offset_min: i32,
    pub hour_lines: bool,
    /// Whether the pointer is on the back button.
    pub button_hovered: bool,
    /// What the back button says when hovered.
    pub back_label: &'a str,
    /// Which way the back button's chevron points: toward the screen.
    pub side: DockSide,
    pub palette: Palette,
}

/// How many laid-out cards are kept, so hovering back and forth between
/// segments does not shape the same text again.
const CARD_CACHE: usize = 12;

/// A rectangle in surface pixels, as damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PixelRect {
    fn from_logical(layout: &Layout, (x, y, w, h): (f32, f32, f32, f32), grow: f32) -> Self {
        let s = layout.scale;
        let left = (x * s - grow).floor() as i32;
        let top = (y * s - grow).floor() as i32;
        let right = ((x + w) * s + grow).ceil() as i32;
        let bottom = ((y + h) * s + grow).ceil() as i32;
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }

    pub fn clamped(self, width: u32, height: u32) -> Self {
        let left = self.x.clamp(0, width as i32);
        let top = self.y.clamp(0, height as i32);
        let right = (self.x + self.width).clamp(left, width as i32);
        let bottom = (self.y + self.height).clamp(top, height as i32);
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }
}

/// What a laid-out card depends on.
#[derive(PartialEq)]
struct CardKey {
    card: Card,
    width: f32,
    scale: f32,
    palette: Palette,
}

type TextSystem = (FontSystem, SwashCache);

/// The font database, built now if the warm-up thread has not delivered it.
fn text_system<'a>(
    text: &'a mut Option<TextSystem>,
    warmup: &mut Option<mpsc::Receiver<TextSystem>>,
) -> &'a mut TextSystem {
    text.get_or_insert_with(|| {
        warmup
            .take()
            .and_then(|loading| loading.recv().ok())
            .unwrap_or_else(|| (FontSystem::new(), SwashCache::new()))
    })
}

/// Loads the system fonts on a thread of their own. Building the font
/// database takes long enough to be felt as lag on the first hover, so it
/// happens while the dock starts instead.
pub fn warm_up_text() -> mpsc::Receiver<TextSystem> {
    let (sender, receiver) = mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("dock-fonts".into())
        .spawn(move || {
            let _ = sender.send((FontSystem::new(), SwashCache::new()));
        });
    receiver
}

/// Owns the font database, which is slow to build and so loaded ahead of time
/// or on first use, and remembers what was last drawn so that a frame only has
/// to redo what changed.
#[derive(Default)]
pub struct Renderer {
    text: Option<TextSystem>,
    warmup: Option<mpsc::Receiver<TextSystem>>,
    cards: Vec<(CardKey, Vec<Block>)>,
    /// Where the last card landed, to be wiped before the next frame.
    last_card: Option<PixelRect>,
}

impl Renderer {
    pub fn with_warmup(warmup: mpsc::Receiver<TextSystem>) -> Self {
        Self {
            warmup: Some(warmup),
            ..Self::default()
        }
    }

    /// Paints a frame onto `pixmap` and says which parts of it changed.
    ///
    /// The pixmap is meant to be kept between frames: only the strip is
    /// repainted and only the old card's patch is cleared, so everything else
    /// stays transparent without being touched.
    pub fn paint(&mut self, layout: &Layout, scene: &Scene, pixmap: &mut Pixmap) -> Vec<PixelRect> {
        let (surface_width, surface_height) = (pixmap.width(), pixmap.height());
        let strip = PixelRect::from_logical(layout, layout.strip_rect(), 0.0);
        let mut damage = vec![strip];

        if let Some(old) = self.last_card.take() {
            clear_rect(pixmap, old);
            damage.push(old);
        }

        let palette = &scene.palette;
        // The two borders sit inside the strip, outermost first. Filling the
        // whole strip with one colour and then the inset rectangles with the
        // next leaves each ring visible on its own.
        let (x, y, w, h) = layout.strip_rect();
        if layout.border_outer > 0.0 {
            fill_rect(pixmap, layout, (x, y, w, h), layout.border_outer_color, 1.0);
        }
        if layout.border_inner > 0.0 {
            let inset = layout.border_outer;
            fill_rect(
                pixmap,
                layout,
                (x + inset, y + inset, w - 2.0 * inset, h - 2.0 * inset),
                layout.border_inner_color,
                1.0,
            );
        }
        fill_rect(
            pixmap,
            layout,
            (
                layout.content_x,
                layout.content_y,
                layout.content_width,
                layout.content_height,
            ),
            palette.background,
            1.0,
        );

        let now = scene.now_minute;
        for (index, segment) in scene.segments.iter().enumerate() {
            let hovered = scene.hovered == Some(index);
            let past = now.is_some_and(|now| (segment.end_min as f32) < now);
            let mut alpha = if past && !hovered { PAST_ALPHA } else { 1.0 };
            if segment.completed {
                alpha *= 0.5;
            }

            let (x, y, w, h) = layout.segment_rect(segment);
            // A hair of background between neighbours keeps back-to-back
            // events from fusing into one bar.
            let inset = if w.max(h) > 4.0 { 0.5 } else { 0.0 };
            let rect = if layout.side.is_vertical() {
                (x, y + inset, w, h - 2.0 * inset)
            } else {
                (x + inset, y, w - 2.0 * inset, h)
            };
            fill_rounded(pixmap, layout, rect, 2.0, segment.color, alpha);

            if hovered {
                stroke_rounded(pixmap, layout, rect, 2.0, palette.text, 1.0);
            } else if scene.pinned == Some(index) {
                stroke_rounded(pixmap, layout, rect, 2.0, palette.accent, 1.0);
            }
        }

        if scene.hour_lines {
            for minute in (1..DAY_MINUTES).filter(|m| (scene.day_offset_min + m) % 60 == 0) {
                let along = layout.along_of(minute as f32) - HOUR_LINE_HEIGHT / 2.0;
                let rect = if layout.side.is_vertical() {
                    (
                        layout.content_x,
                        along,
                        layout.content_width,
                        HOUR_LINE_HEIGHT,
                    )
                } else {
                    (
                        along,
                        layout.content_y,
                        HOUR_LINE_HEIGHT,
                        layout.content_height,
                    )
                };
                fill_rect(pixmap, layout, rect, HOUR_GRAY, 1.0);
            }
        }

        if let Some(minute) = now {
            let along = layout.along_of(minute) - NOW_HEIGHT / 2.0;
            let rect = if layout.side.is_vertical() {
                let along = along.clamp(
                    layout.content_y + layout.header(),
                    layout.content_y + layout.content_height - NOW_HEIGHT,
                );
                (layout.content_x, along, layout.content_width, NOW_HEIGHT)
            } else {
                let along = along.clamp(
                    layout.content_x + layout.header(),
                    layout.content_x + layout.content_width - NOW_HEIGHT,
                );
                (along, layout.content_y, NOW_HEIGHT, layout.content_height)
            };
            fill_rect(pixmap, layout, rect, palette.now, 1.0);
        }

        paint_button(pixmap, layout, scene);

        let card = if scene.button_hovered {
            let card = Card {
                title: scene.back_label.to_owned(),
                when: "Click to switch".to_owned(),
                lines: Vec::new(),
            };
            Some(self.paint_card(layout, palette, &card, layout.axis_start(), pixmap))
        } else {
            // Hovering shows the short card, except over the pinned segment,
            // which keeps showing everything. With nothing hovered, the
            // pinned card stays.
            let shown = match (scene.hovered, scene.pinned) {
                (Some(hovered), Some(pinned)) if hovered == pinned => Some((pinned, true)),
                (Some(hovered), _) => Some((hovered, false)),
                (None, Some(pinned)) => Some((pinned, true)),
                (None, None) => None,
            };
            shown.and_then(|(index, full)| {
                let segment = scene.segments.get(index)?;
                let (x, y, _, _) = layout.segment_rect(segment);
                let anchor = if layout.side.is_vertical() { y } else { x };
                let card = if full { &segment.full } else { &segment.card };
                Some(self.paint_card(layout, palette, card, anchor, pixmap))
            })
        };

        if let Some(card) = card {
            damage.push(card);
            self.last_card = Some(card);
        }
        for rect in &mut damage {
            *rect = rect.clamped(surface_width, surface_height);
        }
        damage
    }

    /// Paints a card beside the strip, aligned with `anchor` along the strip,
    /// and returns the patch it covers.
    fn paint_card(
        &mut self,
        layout: &Layout,
        palette: &Palette,
        card: &Card,
        anchor: f32,
        pixmap: &mut Pixmap,
    ) -> PixelRect {
        let Self {
            text,
            warmup,
            cards,
            ..
        } = self;
        let (font_system, swash) = text_system(text, warmup);

        let inner = (layout.card_size.min(layout.width) - 2.0 * CARD_PADDING).max(10.0);
        let key = CardKey {
            card: card.clone(),
            width: inner,
            scale: layout.scale,
            palette: *palette,
        };
        let index = match cards.iter().position(|(cached, _)| *cached == key) {
            Some(index) => index,
            None => {
                let blocks = card_blocks(font_system, layout, palette, card, inner);
                if cards.len() >= CARD_CACHE {
                    cards.remove(0);
                }
                cards.push((key, blocks));
                cards.len() - 1
            }
        };
        let blocks = &cards[index].1;

        let content: f32 = blocks.iter().map(|block| block.height).sum::<f32>()
            + BLOCK_GAP * blocks.len().saturating_sub(1) as f32;
        let height = content + 2.0 * CARD_PADDING;

        let rect = layout.card_rect(anchor, height);
        let (card_x, card_y, _card_width, height) = rect;

        fill_rounded(pixmap, layout, rect, CARD_RADIUS, palette.card, 1.0);
        stroke_rounded(pixmap, layout, rect, CARD_RADIUS, palette.border, 1.0);

        let mut cursor = card_y + CARD_PADDING;
        for block in blocks {
            if cursor + block.height > card_y + height - CARD_PADDING + 0.5 {
                break;
            }
            draw_block(
                pixmap,
                layout,
                font_system,
                swash,
                block,
                card_x + CARD_PADDING,
                cursor,
            );
            cursor += block.height + BLOCK_GAP;
        }

        PixelRect::from_logical(layout, rect, 2.0)
    }
}

/// Copies a patch of the pixmap into a buffer of the same size and stride.
/// Both are already in the same byte order, so this is one copy per row.
pub fn copy_patch(pixmap: &Pixmap, canvas: &mut [u8], patch: PixelRect) {
    let patch = patch.clamped(pixmap.width(), pixmap.height());
    if patch.width == 0 {
        return;
    }
    let stride = pixmap.width() as usize * 4;
    let (start, len) = (patch.x as usize * 4, patch.width as usize * 4);
    let source = pixmap.data();
    for row in patch.y as usize..(patch.y + patch.height) as usize {
        let at = row * stride + start;
        canvas[at..at + len].copy_from_slice(&source[at..at + len]);
    }
}

/// Wipes a patch of the pixmap back to transparent.
fn clear_rect(pixmap: &mut Pixmap, patch: PixelRect) {
    let patch = patch.clamped(pixmap.width(), pixmap.height());
    if let Some(rect) = Rect::from_xywh(
        patch.x as f32,
        patch.y as f32,
        patch.width as f32,
        patch.height as f32,
    ) {
        let paint = Paint {
            blend_mode: tiny_skia::BlendMode::Clear,
            anti_alias: false,
            ..Paint::default()
        };
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

/// The back button: a rounded square with a chevron pointing into the screen.
fn paint_button(pixmap: &mut Pixmap, layout: &Layout, scene: &Scene) {
    let palette = &scene.palette;
    let rect = layout.button_rect();
    let (fill, ink) = if scene.button_hovered {
        (palette.accent, palette.background)
    } else {
        (palette.border, palette.text)
    };
    fill_rounded(pixmap, layout, rect, 2.0, fill, 1.0);

    let (x, y, w, h) = rect;
    let reach = w.min(h) * 0.22;
    let (centre_x, centre_y) = (x + w / 2.0, y + h / 2.0);
    let (tip, tail_a, tail_b) = match scene.side {
        DockSide::Left => (
            (centre_x + reach, centre_y),
            (centre_x - reach, centre_y - reach * 1.6),
            (centre_x - reach, centre_y + reach * 1.6),
        ),
        DockSide::Right => (
            (centre_x - reach, centre_y),
            (centre_x + reach, centre_y - reach * 1.6),
            (centre_x + reach, centre_y + reach * 1.6),
        ),
        DockSide::Top => (
            (centre_x, centre_y + reach),
            (centre_x - reach * 1.6, centre_y - reach),
            (centre_x + reach * 1.6, centre_y - reach),
        ),
        DockSide::Bottom => (
            (centre_x, centre_y - reach),
            (centre_x - reach * 1.6, centre_y + reach),
            (centre_x + reach * 1.6, centre_y + reach),
        ),
    };
    let s = layout.scale;
    let mut path = PathBuilder::new();
    path.move_to(tail_a.0 * s, tail_a.1 * s);
    path.line_to(tip.0 * s, tip.1 * s);
    path.line_to(tail_b.0 * s, tail_b.1 * s);
    if let Some(path) = path.finish() {
        let stroke = Stroke {
            width: (w / 8.0).max(1.0) * s,
            ..Stroke::default()
        };
        pixmap.stroke_path(
            &path,
            &paint_for(ink, 1.0),
            &stroke,
            Transform::identity(),
            None,
        );
    }
}

/// Draws a laid-out block with its top-left corner at a logical position.
fn draw_block(
    pixmap: &mut Pixmap,
    layout: &Layout,
    font_system: &mut FontSystem,
    swash: &mut SwashCache,
    block: &Block,
    x: f32,
    y: f32,
) {
    let origin = (
        layout.physical(x).round() as i32,
        layout.physical(y).round() as i32,
    );
    block
        .buffer
        .draw(font_system, swash, block.color, |dx, dy, w, h, color| {
            blend_rect(pixmap, origin.0 + dx, origin.1 + dy, w, h, color)
        });
}

struct Block {
    buffer: Buffer,
    color: TextColor,
    /// Logical height of the laid-out text.
    height: f32,
}

fn card_blocks(
    font_system: &mut FontSystem,
    layout: &Layout,
    palette: &Palette,
    card: &Card,
    width: f32,
) -> Vec<Block> {
    let mut blocks = vec![
        text_block(
            font_system,
            layout,
            &card.title,
            TITLE_SIZE,
            Weight::BOLD,
            text_color(palette.text),
            width,
        ),
        text_block(
            font_system,
            layout,
            &card.when,
            BODY_SIZE,
            Weight::NORMAL,
            text_color(palette.muted),
            width,
        ),
    ];

    for line in &card.lines {
        blocks.push(text_block(
            font_system,
            layout,
            line,
            BODY_SIZE,
            Weight::NORMAL,
            text_color(palette.text),
            width,
        ));
    }

    blocks
}

fn text_block(
    font_system: &mut FontSystem,
    layout: &Layout,
    text: &str,
    size: f32,
    weight: Weight,
    color: TextColor,
    width: f32,
) -> Block {
    let metrics = Metrics::new(layout.physical(size), layout.physical(size * LINE_SPACING));
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(layout.physical(width)), None);
    buffer.set_text(
        font_system,
        text,
        &Attrs::new().family(Family::SansSerif).weight(weight),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);

    let lines = buffer.layout_runs().count().max(1);
    let height = lines as f32 * size * LINE_SPACING;

    Block {
        buffer,
        color,
        height,
    }
}

fn text_color(color: Color) -> TextColor {
    let [r, g, b, a] = color.into_rgba8();
    TextColor::rgba(b, g, r, a)
}

fn paint_for(color: Color, alpha: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        (color.b * 255.0).round() as u8,
        (color.g * 255.0).round() as u8,
        (color.r * 255.0).round() as u8,
        (color.a * alpha * 255.0).round() as u8,
    );
    paint.anti_alias = true;
    paint
}

fn fill_rect(
    pixmap: &mut Pixmap,
    layout: &Layout,
    (x, y, w, h): (f32, f32, f32, f32),
    color: Color,
    alpha: f32,
) {
    let s = layout.scale;
    if let Some(rect) = Rect::from_xywh(x * s, y * s, w * s, h * s) {
        let mut paint = paint_for(color, alpha);
        paint.anti_alias = false;
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

fn rounded_path(
    layout: &Layout,
    (x, y, w, h): (f32, f32, f32, f32),
    radius: f32,
) -> Option<tiny_skia::Path> {
    let s = layout.scale;
    let (x, y, w, h) = (x * s, y * s, w * s, h * s);
    let r = (radius * s).min(w / 2.0).min(h / 2.0).max(0.0);

    let mut path = PathBuilder::new();
    path.move_to(x + r, y);
    path.line_to(x + w - r, y);
    path.quad_to(x + w, y, x + w, y + r);
    path.line_to(x + w, y + h - r);
    path.quad_to(x + w, y + h, x + w - r, y + h);
    path.line_to(x + r, y + h);
    path.quad_to(x, y + h, x, y + h - r);
    path.line_to(x, y + r);
    path.quad_to(x, y, x + r, y);
    path.close();
    path.finish()
}

fn fill_rounded(
    pixmap: &mut Pixmap,
    layout: &Layout,
    rect: (f32, f32, f32, f32),
    radius: f32,
    color: Color,
    alpha: f32,
) {
    if let Some(path) = rounded_path(layout, rect, radius) {
        pixmap.fill_path(
            &path,
            &paint_for(color, alpha),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

fn stroke_rounded(
    pixmap: &mut Pixmap,
    layout: &Layout,
    rect: (f32, f32, f32, f32),
    radius: f32,
    color: Color,
    width: f32,
) {
    if let Some(path) = rounded_path(layout, rect, radius) {
        let stroke = Stroke {
            width: width * layout.scale,
            ..Stroke::default()
        };
        pixmap.stroke_path(
            &path,
            &paint_for(color, 1.0),
            &stroke,
            Transform::identity(),
            None,
        );
    }
}

/// Source-over blends a glyph coverage rectangle onto the (premultiplied)
/// pixmap.
fn blend_rect(pixmap: &mut Pixmap, x: i32, y: i32, w: u32, h: u32, color: TextColor) {
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let alpha = color.a() as u32;
    if alpha == 0 {
        return;
    }
    let data = pixmap.data_mut();

    for row in y.max(0)..(y + h as i32).min(height) {
        for column in x.max(0)..(x + w as i32).min(width) {
            let at = (row * width + column) as usize * 4;
            let inverse = 255 - alpha;
            for (offset, channel) in [color.r(), color.g(), color.b()].into_iter().enumerate() {
                let source = channel as u32 * alpha / 255;
                data[at + offset] = (source + data[at + offset] as u32 * inverse / 255) as u8;
            }
            data[at + 3] = (alpha + data[at + 3] as u32 * inverse / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::model::Card;

    fn segment(start_min: i32, end_min: i32) -> Segment {
        let card = Card {
            title: "Standup".into(),
            when: "09:00 – 09:30".into(),
            lines: vec!["Room 4".into()],
        };
        Segment {
            start_min,
            end_min,
            color: Color::from_rgb(0.2, 0.4, 0.9),
            completed: false,
            full: Card {
                lines: vec!["Room 4".into(), "Everything else".into()],
                ..card.clone()
            },
            card,
        }
    }

    #[test]
    fn hour_lines_cross_the_strip_only_when_asked() {
        let layout = Layout::new(&DockConfig::default(), 316, 1440, 1);
        let (width, height) = layout.pixel_size();
        let scene = |hour_lines| Scene {
            segments: &[],
            now_minute: None,
            hovered: None,
            pinned: None,
            day_offset_min: 0,
            hour_lines,
            button_hovered: false,
            back_label: "",
            side: DockSide::Left,
            palette: palette(),
        };
        // One o'clock sits 60 minutes into the day.
        let y = layout.along_of(60.0).floor() as u32;

        let mut off = Pixmap::new(width, height).unwrap();
        Renderer::default().paint(&layout, &scene(false), &mut off);
        let mut on = Pixmap::new(width, height).unwrap();
        Renderer::default().paint(&layout, &scene(true), &mut on);

        assert_ne!(on.pixel(5, y), off.pixel(5, y));
        assert_eq!(on.pixel(5, y + 10), off.pixel(5, y + 10));
        assert_eq!(on.pixel(150, y).unwrap().alpha(), 0);
    }

    #[test]
    fn a_pinned_card_stays_after_the_pointer_leaves_and_is_the_full_one() {
        let layout = Layout::new(&DockConfig::default(), 316, 1440, 1);
        let (width, height) = layout.pixel_size();
        let segments = [segment(600, 630)];
        let scene = |hovered, pinned| Scene {
            segments: &segments,
            now_minute: None,
            hovered,
            pinned,
            day_offset_min: 0,
            hour_lines: false,
            button_hovered: false,
            back_label: "",
            side: DockSide::Left,
            palette: palette(),
        };
        let mut renderer = Renderer::default();
        let mut pixmap = Pixmap::new(width, height).unwrap();

        renderer.paint(&layout, &scene(None, Some(0)), &mut pixmap);
        assert_eq!(pixmap.pixel(150, 615).unwrap().alpha(), 255);
        let shown = renderer.cards.last().unwrap().0.card.clone();
        assert_eq!(shown, segments[0].full);

        renderer.paint(&layout, &scene(Some(0), None), &mut pixmap);
        assert_eq!(renderer.cards.last().unwrap().0.card, segments[0].card);

        renderer.paint(&layout, &scene(None, None), &mut pixmap);
        assert_eq!(pixmap.pixel(150, 615).unwrap().alpha(), 0);
    }

    fn palette() -> Palette {
        Palette::from_theme(&iced::Theme::Light)
    }

    #[test]
    fn the_strip_hugs_the_chosen_edge() {
        let mut config = DockConfig::default();
        let left = Layout::new(&config, 316, 1000, 1);
        assert_eq!((left.strip_x, left.card_rect(0.0, 100.0).0), (0.0, 16.0));

        config.side = DockSide::Right;
        let right = Layout::new(&config, 316, 1000, 1);
        assert_eq!((right.strip_x, right.card_rect(0.0, 100.0).0), (306.0, 0.0));

        config.side = DockSide::Top;
        let top = Layout::new(&config, 1000, 316, 1);
        assert_eq!((top.strip_y, top.card_rect(0.0, 100.0).1), (0.0, 16.0));

        config.side = DockSide::Bottom;
        let bottom = Layout::new(&config, 1000, 316, 1);
        assert_eq!(
            (bottom.strip_y, bottom.card_rect(0.0, 100.0).1),
            (306.0, 200.0)
        );
    }

    #[test]
    fn both_borders_sit_inside_the_strip() {
        let config = DockConfig::default();
        let layout = Layout::new(&config, 316, 1000, 1);

        assert_eq!(layout.border_outer, 1.0);
        assert_eq!(layout.border_inner, 1.0);
        assert_eq!((layout.content_x, layout.content_y), (2.0, 2.0));
        assert_eq!(layout.content_width, 6.0);
    }

    #[test]
    fn a_two_pixel_strip_still_has_a_pixel_for_events() {
        let config = DockConfig {
            width: 2,
            ..DockConfig::default()
        };
        let layout = Layout::new(&config, 316, 1000, 1);

        assert_eq!(layout.content_width, 1.0);
    }

    #[test]
    fn the_two_borders_paint_outermost_first() {
        let config = DockConfig {
            border_outer_color: crate::hex::HexColor(Color::from_rgb(1.0, 0.0, 0.0)),
            border_inner_color: crate::hex::HexColor(Color::from_rgb(0.0, 1.0, 0.0)),
            ..DockConfig::default()
        };
        let layout = Layout::new(&config, 316, 1000, 1);
        let (width, height) = layout.pixel_size();
        let mut pixmap = Pixmap::new(width, height).unwrap();
        let scene = Scene {
            segments: &[],
            now_minute: None,
            hovered: None,
            pinned: None,
            day_offset_min: 0,
            hour_lines: false,
            button_hovered: false,
            back_label: "",
            side: DockSide::Left,
            palette: palette(),
        };

        Renderer::default().paint(&layout, &scene, &mut pixmap);

        // The left edge is the outer border, then the inner one, then the
        // strip's own background.
        let outer = pixmap.pixel(0, 300).unwrap();
        assert_eq!((outer.blue(), outer.green()), (255, 0));
        let inner = pixmap.pixel(1, 300).unwrap();
        assert_eq!((inner.blue(), inner.green()), (0, 255));
        let background = pixmap.pixel(2, 300).unwrap();
        assert_eq!(background.alpha(), 255);
        assert_ne!((background.blue(), background.green()), (255, 0));
    }

    #[test]
    fn a_horizontal_strip_runs_left_to_right_and_its_card_hangs_below() {
        let config = DockConfig {
            side: DockSide::Top,
            ..DockConfig::default()
        };
        let layout = Layout::new(&config, 1000, 316, 1);
        let segment = segment(600, 630);

        let (x, y, w, h) = layout.segment_rect(&segment);
        assert_eq!((y, h), (layout.content_y, layout.content_height));
        assert!(x > layout.button_rect().0 + layout.button_rect().2);
        assert!(w > 0.0);
        assert_eq!(layout.hit(&[segment], f64::from(x) + 1.0, 5.0), Some(0));

        // A top strip puts its card underneath the strip.
        assert_eq!(layout.card_rect(x, 100.0).1, 16.0);
    }

    #[test]
    fn hits_are_limited_to_the_strip_and_the_segment() {
        let layout = Layout::new(&DockConfig::default(), 316, 1440, 1);
        let segments = [segment(60, 120)];

        assert_eq!(layout.hit(&segments, 5.0, 90.0), Some(0));
        assert_eq!(layout.hit(&segments, 5.0, 140.0), None);
        assert_eq!(layout.hit(&segments, 50.0, 90.0), None);
    }

    #[test]
    fn the_back_button_sits_at_the_start_and_inside_the_strip_width() {
        let layout = Layout::new(&DockConfig::default(), 316, 1440, 1);
        let (x, y, w, h) = layout.button_rect();

        assert_eq!((x, y, w, h), (2.0, 2.0, 6.0, 6.0));
        assert!(layout.hit_button(5.0, 5.0));
        assert!(!layout.hit_button(5.0, 11.0));
        assert!(!layout.hit_button(50.0, 5.0));
    }

    #[test]
    fn the_day_starts_below_the_button() {
        let layout = Layout::new(&DockConfig::default(), 316, 1440, 1);
        let midnight = segment(0, 30);

        let (_, top, _, _) = layout.segment_rect(&midnight);

        assert!(top >= layout.button_rect().3);
        assert_eq!(layout.hit(&[midnight], 5.0, 5.0), None);
    }

    #[test]
    fn hovering_the_back_button_shows_what_it_opens() {
        let layout = Layout::new(&DockConfig::default(), 316, 1440, 1);
        let (width, height) = layout.pixel_size();
        let mut pixmap = Pixmap::new(width, height).unwrap();
        let scene = Scene {
            segments: &[],
            now_minute: None,
            hovered: None,
            pinned: None,
            day_offset_min: 0,
            hour_lines: false,
            button_hovered: true,
            back_label: "Open the full window",
            side: DockSide::Left,
            palette: palette(),
        };

        Renderer::default().paint(&layout, &scene, &mut pixmap);

        let accent = scene.palette.accent.into_rgba8();
        let corner = pixmap.pixel(3, 5).unwrap();
        assert_eq!(corner.alpha(), 255);
        assert_eq!(corner.blue(), accent[0]);
        assert_eq!(pixmap.pixel(150, 20).unwrap().alpha(), 255);
        assert_eq!(pixmap.pixel(150, 700).unwrap().alpha(), 0);
    }

    #[test]
    fn tiny_segments_stay_hoverable() {
        let layout = Layout::new(&DockConfig::default(), 316, 1440, 1);
        let (_, _, _, height) = layout.segment_rect(&segment(60, 61));

        assert!(height >= MIN_SEGMENT_HEIGHT);
    }

    #[test]
    fn gaps_are_background_and_the_rest_of_the_surface_is_clear() {
        let config = DockConfig::default();
        let layout = Layout::new(&config, 316, 1440, 1);
        let (width, height) = layout.pixel_size();
        let mut pixmap = Pixmap::new(width, height).unwrap();
        let scene = Scene {
            segments: &[segment(60, 120)],
            now_minute: Some(600.0),
            hovered: None,
            pinned: None,
            day_offset_min: 0,
            hour_lines: false,
            button_hovered: false,
            back_label: "Open the full window",
            side: DockSide::Left,
            palette: palette(),
        };

        Renderer::default().paint(&layout, &scene, &mut pixmap);

        let at = |x: u32, y: u32| pixmap.pixel(x, y).unwrap();
        // A gap in the strip is the opaque background colour.
        let gap = at(5, 300);
        assert_eq!(gap.alpha(), 255);
        let background = scene.palette.background.into_rgba8();
        assert_eq!(
            (gap.blue(), gap.green(), gap.red()),
            (background[0], background[1], background[2])
        );
        // The segment is its own colour.
        assert_ne!(at(5, 90), gap);
        // The "now" line is the danger colour.
        let now = scene.palette.now.into_rgba8();
        let now_y = layout.along_of(600.0).floor() as u32;
        assert_eq!(at(5, now_y).blue(), now[0]);
        // Beyond the strip nothing is painted, so clicks and eyes pass through.
        assert_eq!(at(100, 300).alpha(), 0);
    }

    #[test]
    fn hovering_draws_a_card_beside_the_segment() {
        let config = DockConfig::default();
        let layout = Layout::new(&config, 316, 1440, 1);
        let (width, height) = layout.pixel_size();
        let mut pixmap = Pixmap::new(width, height).unwrap();
        let scene = Scene {
            segments: &[segment(600, 630)],
            now_minute: None,
            hovered: Some(0),
            pinned: None,
            day_offset_min: 0,
            hour_lines: false,
            button_hovered: false,
            back_label: "Open the full window",
            side: DockSide::Left,
            palette: palette(),
        };

        Renderer::default().paint(&layout, &scene, &mut pixmap);

        assert_eq!(pixmap.pixel(150, 615).unwrap().alpha(), 255);
        assert_eq!(pixmap.pixel(150, 100).unwrap().alpha(), 0);
    }

    #[test]
    fn moving_off_a_card_clears_it_and_only_damages_what_changed() {
        let layout = Layout::new(&DockConfig::default(), 316, 1440, 1);
        let (width, height) = layout.pixel_size();
        let mut pixmap = Pixmap::new(width, height).unwrap();
        let segments = [segment(600, 630)];
        let scene = |hovered| Scene {
            segments: &segments,
            now_minute: None,
            hovered,
            pinned: None,
            day_offset_min: 0,
            hour_lines: false,
            button_hovered: false,
            back_label: "Open the full window",
            side: DockSide::Left,
            palette: palette(),
        };
        let mut renderer = Renderer::default();

        let covers = |damage: &[PixelRect], x: i32, y: i32| {
            damage
                .iter()
                .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
        };

        let hovered = renderer.paint(&layout, &scene(Some(0)), &mut pixmap);
        assert_eq!(pixmap.pixel(150, 615).unwrap().alpha(), 255);
        assert!(covers(&hovered, 150, 615));

        let left = renderer.paint(&layout, &scene(None), &mut pixmap);
        assert_eq!(pixmap.pixel(150, 615).unwrap().alpha(), 0);
        assert!(covers(&left, 150, 615));
        assert!(!covers(&left, 150, 100));

        let idle = renderer.paint(&layout, &scene(None), &mut pixmap);
        assert_eq!(idle.len(), 1);
        assert!(!covers(&idle, 150, 615));
    }

    #[test]
    fn the_same_card_is_only_shaped_once() {
        let layout = Layout::new(&DockConfig::default(), 316, 1440, 1);
        let (width, height) = layout.pixel_size();
        let mut pixmap = Pixmap::new(width, height).unwrap();
        let segments = [segment(600, 630), segment(700, 730)];
        let mut renderer = Renderer::default();

        for hovered in [0, 1, 0, 1, 0] {
            let scene = Scene {
                segments: &segments,
                now_minute: None,
                hovered: Some(hovered),
                pinned: None,
                day_offset_min: 0,
                hour_lines: false,
                button_hovered: false,
                back_label: "Open the full window",
                side: DockSide::Left,
                palette: palette(),
            };
            renderer.paint(&layout, &scene, &mut pixmap);
        }

        // Both segments carry identical card text, so one entry serves both.
        assert_eq!(renderer.cards.len(), 1);
    }

    #[test]
    fn copying_a_patch_leaves_the_rest_of_the_buffer_alone() {
        let mut pixmap = Pixmap::new(4, 2).unwrap();
        pixmap.data_mut().copy_from_slice(&[
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, //
            17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
        ]);
        let mut canvas = [0u8; 32];
        let patch = PixelRect {
            x: 1,
            y: 1,
            width: 2,
            height: 1,
        };

        copy_patch(&pixmap, &mut canvas, patch);

        assert_eq!(&canvas[..20], &[0; 20]);
        assert_eq!(&canvas[20..28], &[21, 22, 23, 24, 25, 26, 27, 28]);
        assert_eq!(&canvas[28..], &[0; 4]);
    }
}
