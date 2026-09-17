//! The routing panel: what a track feeds, and what feeds it.
//!
//! REAPER puts this behind the same button — the little routing widget
//! on a strip — and so does this window. The difference is that the
//! button used to toggle the parent send and nothing else, because "the
//! routing widget opens a matrix that does not exist". This is that
//! matrix, for one track at a time.
//!
//! # One track, not a grid
//!
//! A routing MATRIX is every track against every track, and in a
//! two-hundred-track session it is forty thousand cells nobody can
//! read. What an engineer actually asks is about one track: what is
//! this feeding, how much, and who is feeding it. So the panel is that
//! question, opened on the track whose button was pressed.
//!
//! # Why the layout is a list of lines
//!
//! Everything here — the title, the parent-send row, each send, each
//! receive — is one row of the same height, and the panel is as tall as
//! it has rows. That makes the geometry a single index into a list,
//! which is what lets the hit test and the drawing share it exactly:
//! [`lines`] is asked once by each, so a click cannot land on a row the
//! paint drew somewhere else.

use vello::kurbo::Rect;

/// How wide the panel is.
///
/// Wide enough for a destination name, a mode, a mute, a level, a pan
/// and a way to remove it, all on one line. A narrower panel would
/// stack them, and a send whose controls are on two lines is a send you
/// have to read twice.
pub const WIDTH: f64 = 300.0;

/// How tall one row is.
pub const ROW_H: f64 = 20.0;

/// The gap left inside the panel's edge.
pub const PAD: f64 = 6.0;

/// One line of the panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Line {
    /// The track's name, and the way out.
    Title,
    /// Whether the track feeds its parent at all.
    Parent,
    /// The `SENDS` heading.
    SendsLabel,
    /// One send, by its position in the list.
    Send(usize),
    /// The `RECEIVES` heading.
    ReceivesLabel,
    /// One receive, by its position in the list.
    Receive(usize),
    /// Said where a list would be if it had anything in it.
    ///
    /// A heading with nothing under it reads as a panel that failed to
    /// load. "Nothing" is a shorter read than an empty space.
    Nothing,
}

/// What part of a send row a point is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    /// The destination's name.
    Name,
    /// Post-fader, pre-FX, post-FX. Clicking walks them.
    Mode,
    /// Whether the send passes anything.
    Mute,
    /// How much goes down it — dragged.
    Level,
    /// Where it sits — dragged.
    Pan,
    /// Take it away.
    Remove,
}

/// What the pointer is on in the panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spot {
    /// Close it.
    Close,
    /// The parent-send toggle.
    Parent,
    /// A part of one of the sends.
    Send { index: usize, part: Part },
    /// A receive — nothing to press yet, but a press on one must not
    /// fall through to the window behind the panel.
    Receive { index: usize },
    /// Inside the panel, on nothing in particular.
    Nowhere,
}

/// The lines a panel with this many sends and receives has, in order.
///
/// The single source both the drawing and the hit test read, which is
/// what makes "you clicked what you saw" true by construction rather
/// than by two lists being kept in step.
#[must_use]
pub fn lines(sends: usize, receives: usize) -> Vec<Line> {
    let mut lines = vec![Line::Title, Line::Parent, Line::SendsLabel];
    if sends == 0 {
        lines.push(Line::Nothing);
    }
    lines.extend((0..sends).map(Line::Send));
    lines.push(Line::ReceivesLabel);
    if receives == 0 {
        lines.push(Line::Nothing);
    }
    lines.extend((0..receives).map(Line::Receive));
    lines
}

/// How tall a panel with this many lines is.
#[must_use]
pub fn height(lines: usize) -> f64 {
    crate::num::coord(lines).mul_add(ROW_H, PAD * 2.0)
}

/// Where the panel goes when it is opened at a point.
///
/// Kept inside the window, because a panel opened from a strip near the
/// right edge would otherwise hang off it — and the controls that fall
/// off are the ones on the right, which is every control that does
/// anything.
#[must_use]
pub fn anchored(at: (f64, f64), frame: (f64, f64), lines: usize) -> Rect {
    let (width, height) = (WIDTH, height(lines));
    let x = at.0.min(frame.0 - width).max(0.0);
    let y = at.1.min(frame.1 - height).max(0.0);
    Rect::new(x, y, x + width, y + height)
}

/// The box of one line.
#[must_use]
pub fn row(panel: Rect, index: usize) -> Rect {
    let top = crate::num::coord(index).mul_add(ROW_H, panel.y0 + PAD);
    Rect::new(panel.x0 + PAD, top, panel.x1 - PAD, top + ROW_H)
}

/// The columns of a send row, in the row's own box.
///
/// One place, read by the paint and the hit test — the same reason
/// [`lines`] is one place.
#[must_use]
pub fn column(row: Rect, part: Part) -> Rect {
    let x = row.x0;
    let (from, to) = match part {
        Part::Name => (0.0, 108.0),
        Part::Mode => (112.0, 148.0),
        Part::Mute => (152.0, 170.0),
        Part::Level => (174.0, 232.0),
        Part::Pan => (236.0, 264.0),
        Part::Remove => (270.0, 288.0),
    };
    Rect::new(x + from, row.y0 + 2.0, x + to, row.y1 - 2.0)
}

/// What is under a point in a panel.
///
/// `None` outside it — which is how a press elsewhere closes it rather
/// than being swallowed.
#[must_use]
pub fn spot_at(panel: Rect, sends: usize, receives: usize, x: f64, y: f64) -> Option<Spot> {
    if x < panel.x0 || x >= panel.x1 || y < panel.y0 || y >= panel.y1 {
        return None;
    }
    let lines = lines(sends, receives);
    let index = ((y - panel.y0 - PAD) / ROW_H).floor();
    if index < 0.0 {
        return Some(Spot::Nowhere);
    }
    let Some(line) = lines.get(crate::num::index(index)) else {
        return Some(Spot::Nowhere);
    };
    let row = row(panel, crate::num::index(index));
    Some(match *line {
        Line::Title => {
            // The way out is the right end of the title, where every
            // window's is.
            if x >= row.x1 - ROW_H {
                Spot::Close
            } else {
                Spot::Nowhere
            }
        }
        Line::Parent => Spot::Parent,
        Line::Send(index) => match part_at(row, x) {
            Some(part) => Spot::Send { index, part },
            None => Spot::Nowhere,
        },
        Line::Receive(index) => Spot::Receive { index },
        Line::SendsLabel | Line::ReceivesLabel | Line::Nothing => Spot::Nowhere,
    })
}

/// Which column of a send row an x is in.
fn part_at(row: Rect, x: f64) -> Option<Part> {
    [
        Part::Mode,
        Part::Mute,
        Part::Level,
        Part::Pan,
        Part::Remove,
        Part::Name,
    ]
    .into_iter()
    .find(|part| {
        let cell = column(row, *part);
        x >= cell.x0 && x < cell.x1
    })
}

/// The next send mode, for a click on the mode cell.
///
/// Post-fader, pre-FX, post-FX and back. REAPER's own three, in the
/// order anybody wants them: the default first, then the one a cue mix
/// needs, then the odd one.
#[must_use]
pub const fn next_mode(mode: daw_proto::routing::SendMode) -> daw_proto::routing::SendMode {
    use daw_proto::routing::SendMode as M;
    match mode {
        M::PostFader => M::PreFx,
        M::PreFx => M::PostFx,
        M::PostFx => M::PostFader,
    }
}

/// How a mode is written in the cell.
#[must_use]
pub const fn mode_name(mode: daw_proto::routing::SendMode) -> &'static str {
    use daw_proto::routing::SendMode as M;
    match mode {
        M::PostFader => "POST",
        M::PreFx => "PRE FX",
        M::PostFx => "POST FX",
    }
}

/// Where a drag along a level cell puts the send, as a gain.
///
/// The cell is the same scale the fader uses, so a send at unity sits
/// where a fader at unity does and the two read against each other.
/// Clamped to the cell: a drag that ran off the end would otherwise
/// keep going, and a send at +30 dB is not a thing anybody meant.
#[must_use]
pub fn level_at(cell: Rect, x: f64) -> f64 {
    let fraction = ((x - cell.x0) / cell.width().max(1.0)).clamp(0.0, 1.0);
    crate::engine::db_to_gain(daw_theme_art::paint::tcp::fader_db(fraction))
}

/// How much of a level cell a gain fills — the inverse of [`level_at`],
/// for drawing the one the other reads.
#[must_use]
pub fn level_fraction(gain: f64) -> f64 {
    daw_theme_art::paint::tcp::gain_norm(gain).clamp(0.0, 1.0)
}

/// How much of a pan cell sits left of the marker.
#[must_use]
pub fn pan_fraction(pan: f64) -> f64 {
    pan.mul_add(0.5, 0.5).clamp(0.0, 1.0)
}

/// Where a drag along a pan cell puts the send.
#[must_use]
pub fn pan_at(cell: Rect, x: f64) -> f64 {
    let fraction = ((x - cell.x0) / cell.width().max(1.0)).clamp(0.0, 1.0);
    fraction.mul_add(2.0, -1.0)
}

/// Draw the panel over everything.
///
/// In the live pass, not the recording: a send's level moves while you
/// drag it, and the panel is open for exactly as long as somebody is
/// changing something. Re-recording the mixer per frame to show that
/// would cost the whole mixer.
pub fn paint(
    painter: &mut impl anyrender::PaintScene,
    palette: &crate::arrangement::Palette,
    font: &crate::text::Font,
    open: Open<'_>,
    panel: Rect,
) {
    use vello::peniko::Fill;

    const SIZE: f32 = 9.0;
    let Open {
        guid,
        name,
        parent_send,
        wiring,
        names,
    } = open;
    let (sends, receives) = wiring.map_or((&[][..], &[][..]), |w| {
        (w.sends.as_slice(), w.receives.as_slice())
    });

    fill(painter, palette.surface, panel);
    fill(
        painter,
        palette.accent,
        Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + 2.0),
    );

    for (index, line) in lines(sends.len(), receives.len()).iter().enumerate() {
        let row = row(panel, index);
        let text_y = row.y0 + ROW_H / 2.0 + f64::from(SIZE) / 3.0;
        match *line {
            Line::Title => {
                crate::tcp::glyphs(painter, font, palette.text, name, row.x0, text_y, 10.0);
                crate::tcp::glyphs(
                    painter,
                    font,
                    palette.text_dim,
                    "×",
                    row.x1 - ROW_H / 2.0,
                    text_y,
                    11.0,
                );
            }
            Line::Parent => {
                let lit = if parent_send {
                    palette.accent
                } else {
                    palette.tcp_button
                };
                fill(
                    painter,
                    lit,
                    Rect::new(row.x0, row.y0 + 4.0, row.x0 + 12.0, row.y1 - 4.0),
                );
                crate::tcp::glyphs(
                    painter,
                    font,
                    palette.text_dim,
                    "TO PARENT",
                    row.x0 + 18.0,
                    text_y,
                    SIZE,
                );
            }
            Line::SendsLabel | Line::ReceivesLabel => {
                let label = if *line == Line::SendsLabel {
                    "SENDS"
                } else {
                    "RECEIVES"
                };
                crate::tcp::glyphs(
                    painter,
                    font,
                    palette.text_faint,
                    label,
                    row.x0,
                    text_y,
                    8.0,
                );
                fill(
                    painter,
                    palette.tcp_rule,
                    Rect::new(row.x0, row.y1 - 1.0, row.x1, row.y1),
                );
            }
            // A heading with nothing under it reads as a failed load.
            Line::Nothing => crate::tcp::glyphs(
                painter,
                font,
                palette.text_faint,
                "nothing",
                row.x0 + 8.0,
                text_y,
                SIZE,
            ),
            Line::Send(index) => {
                let Some(route) = sends.get(index) else {
                    continue;
                };
                let ink = if route.muted {
                    palette.text_faint
                } else {
                    palette.text
                };
                let to = far_end(route, guid, names);
                cell_text(painter, font, ink, column(row, Part::Name), &to, SIZE);
                cell_text(
                    painter,
                    font,
                    palette.text_dim,
                    column(row, Part::Mode),
                    mode_name(route.send_mode),
                    8.0,
                );
                let mute = column(row, Part::Mute);
                fill(
                    painter,
                    if route.muted {
                        palette.mute
                    } else {
                        palette.tcp_button
                    },
                    mute,
                );
                cell_text(painter, font, palette.text, mute, "M", 8.0);
                bar(
                    painter,
                    palette,
                    column(row, Part::Level),
                    level_fraction(route.volume),
                    palette.accent,
                );
                marker(
                    painter,
                    palette,
                    column(row, Part::Pan),
                    pan_fraction(route.pan),
                );
                cell_text(
                    painter,
                    font,
                    palette.text_dim,
                    column(row, Part::Remove),
                    "×",
                    10.0,
                );
            }
            // A receive is the far end of somebody else's send, so it
            // is shown and not touched: the thing that owns it is that
            // send, on that track.
            Line::Receive(index) => {
                let Some(route) = receives.get(index) else {
                    continue;
                };
                let from = far_end(route, guid, names);
                cell_text(
                    painter,
                    font,
                    palette.text_dim,
                    column(row, Part::Name),
                    &from,
                    SIZE,
                );
                bar(
                    painter,
                    palette,
                    column(row, Part::Level),
                    level_fraction(route.volume),
                    palette.text_faint,
                );
            }
        }
    }

    // A hairline all the way round, last, so the panel reads as one
    // thing over the mixer rather than as a hole in it.
    for edge in [
        Rect::new(panel.x0, panel.y0, panel.x0 + 1.0, panel.y1),
        Rect::new(panel.x1 - 1.0, panel.y0, panel.x1, panel.y1),
        Rect::new(panel.x0, panel.y1 - 1.0, panel.x1, panel.y1),
    ] {
        painter.fill(
            Fill::NonZero,
            vello::kurbo::Affine::IDENTITY,
            palette.tcp_rule,
            None,
            &edge,
        );
    }
}

/// Everything the panel draws, gathered by the caller.
///
/// A borrow rather than the track itself: the panel is about ONE
/// track's routing, and handing it the whole track would let it start
/// drawing things the strip already draws.
#[derive(Clone, Copy)]
pub struct Open<'a> {
    /// The track the panel is about — needed to tell which end of a
    /// receive is the other one. See [`crate::routes::partner`].
    pub guid: &'a str,
    pub name: &'a str,
    pub parent_send: bool,
    /// `None` while the read is still out — which is not the same as a
    /// track with no routing, and must not draw as one.
    pub wiring: Option<&'a crate::routes::Wiring>,
    /// A track's name, by guid. The panel names the far end of a route,
    /// and a route carries a guid — the names are the window's.
    pub names: &'a dyn Fn(&str) -> Option<String>,
}

/// What to call the other end of a route.
///
/// The guid that is not this track's, resolved to a name — falling back
/// to the name the route carried, and then to a hardware output, which
/// has no track to name at all.
fn far_end(
    route: &daw_proto::routing::TrackRoute,
    mine: &str,
    names: &dyn Fn(&str) -> Option<String>,
) -> String {
    crate::routes::partner(route, mine)
        .and_then(names)
        .or_else(|| route.dest_track_name.clone())
        .or_else(|| route.hw_output_name.clone())
        .unwrap_or_else(|| "—".to_owned())
}

fn fill(painter: &mut impl anyrender::PaintScene, color: vello::peniko::Color, rect: Rect) {
    use anyrender::PaintScene as _;
    painter.fill(
        vello::peniko::Fill::NonZero,
        vello::kurbo::Affine::IDENTITY,
        color,
        None,
        &rect,
    );
}

/// Text centred in a cell, trimmed to it.
fn cell_text(
    painter: &mut impl anyrender::PaintScene,
    font: &crate::text::Font,
    ink: vello::peniko::Color,
    cell: Rect,
    body: &str,
    size: f32,
) {
    let body = fit(font, body, size, cell.width() - 2.0);
    let width = font.width(body, size);
    crate::tcp::glyphs(
        painter,
        font,
        ink,
        body,
        cell.x0 + (cell.width() - width) / 2.0,
        cell.y0 + cell.height() / 2.0 + f64::from(size) / 3.0,
        size,
    );
}

/// As much of a name as fits, cut rather than overflowing into the next
/// control.
fn fit<'a>(font: &crate::text::Font, body: &'a str, size: f32, room: f64) -> &'a str {
    let mut body = body;
    while !body.is_empty() && font.width(body, size) > room {
        let mut end = body.len().saturating_sub(1);
        while end > 0 && !body.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        body = &body[..end];
    }
    body
}

/// A level, drawn as the share of its cell it fills.
fn bar(
    painter: &mut impl anyrender::PaintScene,
    palette: &crate::arrangement::Palette,
    cell: Rect,
    fraction: f64,
    ink: vello::peniko::Color,
) {
    fill(painter, palette.tcp_meter_well, cell);
    let to = fraction.clamp(0.0, 1.0).mul_add(cell.width(), cell.x0);
    fill(painter, ink, Rect::new(cell.x0, cell.y0, to, cell.y1));
}

/// A pan, drawn as a mark in its cell with the centre shown.
fn marker(
    painter: &mut impl anyrender::PaintScene,
    palette: &crate::arrangement::Palette,
    cell: Rect,
    fraction: f64,
) {
    fill(painter, palette.tcp_meter_well, cell);
    let middle = cell.x0 + cell.width() / 2.0;
    fill(
        painter,
        palette.tcp_rule,
        Rect::new(middle, cell.y0, middle + 1.0, cell.y1),
    );
    let at = fraction
        .clamp(0.0, 1.0)
        .mul_add(cell.width() - 3.0, cell.x0);
    fill(
        painter,
        palette.pan,
        Rect::new(at, cell.y0, at + 3.0, cell.y1),
    );
}

#[cfg(test)]
mod tests {
    use super::{Line, Part, Spot, anchored, column, height, level_at, lines, next_mode, spot_at};

    /// The lines are the panel: a heading with nothing under it says
    /// so, rather than leaving a gap that reads as a failed load.
    #[test]
    fn an_empty_list_says_nothing_rather_than_showing_nothing() {
        let lines = lines(0, 0);
        assert_eq!(
            lines,
            vec![
                Line::Title,
                Line::Parent,
                Line::SendsLabel,
                Line::Nothing,
                Line::ReceivesLabel,
                Line::Nothing,
            ]
        );
    }

    /// Sends come before receives, and each is numbered from its own
    /// list — a receive is not "send 3".
    #[test]
    fn each_list_is_numbered_from_its_own_top() {
        let lines = lines(2, 1);
        assert_eq!(lines[3], Line::Send(0));
        assert_eq!(lines[4], Line::Send(1));
        assert_eq!(lines[6], Line::Receive(0));
    }

    /// Every part of a send row is hit where it is drawn. The whole
    /// point of one column table read by both.
    #[test]
    fn a_send_control_is_hit_where_it_is_drawn() {
        let panel = anchored((0.0, 0.0), (1920.0, 1080.0), lines(1, 0).len());
        let row = super::row(panel, 3);
        for part in [
            Part::Name,
            Part::Mode,
            Part::Mute,
            Part::Level,
            Part::Pan,
            Part::Remove,
        ] {
            let cell = column(row, part);
            let hit = spot_at(panel, 1, 0, cell.x0 + 1.0, cell.y0 + 1.0);
            assert_eq!(
                hit,
                Some(Spot::Send { index: 0, part }),
                "{part:?} was not hit in its own cell"
            );
        }
    }

    /// A press outside is not the panel's — that is what closes it.
    #[test]
    fn outside_is_nobodys() {
        let panel = anchored((100.0, 100.0), (1920.0, 1080.0), 6);
        assert_eq!(spot_at(panel, 0, 0, 99.0, 150.0), None);
        assert_eq!(spot_at(panel, 0, 0, 150.0, 99.0), None);
        // But a press on a heading inside it IS the panel's, or it
        // would fall through and close the thing it landed on.
        assert_eq!(spot_at(panel, 0, 0, 150.0, 150.0), Some(Spot::Nowhere));
    }

    /// Opened near an edge, the panel comes back inside it. The
    /// controls that would hang off are the ones on the right, which is
    /// every control that does anything.
    #[test]
    fn a_panel_opened_at_the_edge_stays_on_screen() {
        let panel = anchored((1900.0, 1070.0), (1920.0, 1080.0), 8);
        assert!(panel.x1 <= 1920.0, "it ran off to {}", panel.x1);
        assert!(panel.y1 <= 1080.0, "it ran off to {}", panel.y1);
        assert!((panel.width() - super::WIDTH).abs() < 1e-9, "it was shrunk");
        assert!((panel.height() - height(8)).abs() < 1e-9);
    }

    /// The panel draws, and draws MORE when there is something in it.
    ///
    /// A smoke test with teeth: a paint that panics takes the window
    /// with it, and a paint that silently draws nothing is a panel that
    /// opens as a grey box. Counting commands catches both, without
    /// pinning a picture that every colour change would have to
    /// re-bless.
    #[test]
    fn a_panel_with_sends_in_it_draws_more_than_an_empty_one() {
        use daw_proto::routing::{RouteType, TrackRoute};

        let palette = crate::arrangement::Palette::from_theme(&daw_ui::theming::Theme::dark());
        let font = crate::text::Font::embedded().expect("the embedded font");
        let send = |to: &str| TrackRoute {
            route_type: RouteType::Send,
            dest_track_name: Some(to.to_owned()),
            dest_track_guid: Some(to.to_owned()),
            ..TrackRoute::default()
        };
        let wiring = crate::routes::Wiring {
            sends: vec![send("VERB"), send("DELAY")],
            receives: vec![send("BUS")],
        };

        let drawn = |wiring: Option<&crate::routes::Wiring>| {
            let (sends, receives) = wiring.map_or((0, 0), |w| (w.sends.len(), w.receives.len()));
            let panel = anchored((40.0, 40.0), (1920.0, 1080.0), lines(sends, receives).len());
            let mut scene = anyrender::Scene::new();
            super::paint(
                &mut scene,
                &palette,
                &font,
                super::Open {
                    guid: "kick",
                    name: "KICK",
                    parent_send: true,
                    wiring,
                    names: &|guid| Some(guid.to_uppercase()),
                },
                panel,
            );
            scene.commands.len()
        };

        let empty = drawn(None);
        assert!(empty > 0, "an open panel drew nothing at all");
        assert!(
            drawn(Some(&wiring)) > empty,
            "two sends and a receive drew no more than an empty panel"
        );
    }

    /// The modes cycle, and they come back.
    #[test]
    fn the_modes_are_a_ring() {
        use daw_proto::routing::SendMode as M;
        assert_eq!(next_mode(M::PostFader), M::PreFx);
        assert_eq!(next_mode(next_mode(next_mode(M::PostFader))), M::PostFader);
    }

    /// A level drag is the fader's scale, and it stops at both ends.
    #[test]
    fn a_level_drag_stays_between_silence_and_the_top() {
        let panel = anchored((0.0, 0.0), (1920.0, 1080.0), lines(1, 0).len());
        let cell = column(super::row(panel, 3), Part::Level);
        let quiet = level_at(cell, cell.x0 - 500.0);
        let loud = level_at(cell, cell.x1 + 500.0);
        assert!(quiet <= loud);
        assert!(quiet >= 0.0, "a send went below silence");
        assert!(loud <= 4.0, "a send went to {loud}, which is not a send");
        // And what is drawn is what was set: the cell a drag reads is
        // the cell the level fills.
        let middle = level_at(cell, cell.x0 + cell.width() / 2.0);
        let drawn = super::level_fraction(middle);
        assert!((drawn - 0.5).abs() < 0.02, "half a cell drew as {drawn}");
    }
}
