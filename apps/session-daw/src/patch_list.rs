//! The Patch List view: source kinds against inputs, grouped by
//! performer.
//!
//! Read-only here (spec #48, `flow.patch-list.plan`): the table is a
//! projection of the album's list resolved against the active studio
//! profile, drawn the way tracking sorts — each performer a header,
//! their rig under it, one row per channel or mic — with the headphone
//! buses at the bottom and every role the room does not have marked
//! rather than dropped. Editing a row, applying the plan and the
//! per-session override are #57.
//!
//! The picture is recorded the way every other view in this window is:
//! one `PaintScene`, explicit geometry, the embedded font, no CSS and
//! no layout engine — so the bench can draw it headlessly and the
//! committed PNG is the fixture.

use anyrender::PaintScene;
use patch_list::profile::Resolved;
use patch_list::{PatchList, StudioProfile};
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};

use crate::arrangement::Palette;
use crate::text::Font;

/// Whether the room resolved a row's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// The profile has the role.
    Resolved,
    /// It does not — shown, never an error.
    Unresolved,
}

/// One channel or mic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The kind of source (`guitar`, `drums`).
    pub kind: String,
    /// The rig key as the list writes it (`di`, `kick/in`).
    pub key: String,
    /// The input role, or the MIDI device for a MIDI entry.
    pub role: String,
    /// What the room makes of it, for the Input column.
    pub input: String,
    pub mark: Mark,
}

/// One performer's rows, under their header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerformerRows {
    pub name: String,
    pub rows: Vec<Row>,
}

/// One headphone bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusRow {
    pub name: String,
    /// Who it is for, joined for the column.
    pub audience: String,
    /// The bus role.
    pub role: String,
    /// The output pair, or `unresolved`.
    pub input: String,
    pub mark: Mark,
}

/// The whole view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The room the table was resolved against.
    pub studio: String,
    pub performers: Vec<PerformerRows>,
    pub buses: Vec<BusRow>,
    /// How many rows and buses the room could not resolve.
    pub unresolved: usize,
}

/// The fixture album, for the render fixture and the tests.
const FIXTURE_ALBUM: &str =
    include_str!("../../../features/patch-list/fixtures/album/patch-list.styx");
/// And the fixture room.
const FIXTURE_ROOM: &str =
    include_str!("../../../features/patch-list/fixtures/studios/golden-room.styx");

impl Table {
    /// Build the table from a list and a room.
    ///
    /// A list the room double-books is not a table: validation is the
    /// one refusal, and the caller shows the error instead. Everything
    /// else — a role the room lacks, a bus it has no output for — is a
    /// marked row.
    ///
    /// # Errors
    ///
    /// When two entries name one role the profile does not share.
    // r[impl flow.patch-list.plan]
    // r[impl flow.patch-list.studio-profiles]
    pub fn build(
        studio: &str,
        list: &PatchList,
        profile: &StudioProfile,
    ) -> Result<Self, patch_list::Error> {
        let plan = patch_list::validate(list, profile)?;
        let mut performers: Vec<PerformerRows> = Vec::new();
        for entry in &plan.entries {
            let row = Row {
                kind: entry.lowered.kind.clone(),
                key: entry.lowered.key.clone(),
                role: match &entry.lowered.entry {
                    patch_list::Entry::Role(role) => role.clone(),
                    patch_list::Entry::Midi { device, .. } => device.clone(),
                },
                input: describe(&entry.resolved),
                mark: mark(&entry.resolved),
            };
            match performers
                .iter_mut()
                .find(|p| p.name == entry.lowered.performer)
            {
                Some(performer) => performer.rows.push(row),
                None => performers.push(PerformerRows {
                    name: entry.lowered.performer.clone(),
                    rows: vec![row],
                }),
            }
        }
        let buses = plan
            .buses
            .iter()
            .map(|bus| BusRow {
                name: bus.name.clone(),
                audience: bus.bus.audience.join(", "),
                role: bus.bus.output.clone(),
                input: describe(&bus.resolved),
                mark: mark(&bus.resolved),
            })
            .collect();
        Ok(Self {
            studio: studio.to_owned(),
            performers,
            buses,
            unresolved: plan.unresolved(),
        })
    }

    /// The table for a project: the album file found by walking up from
    /// its directory, resolved against the machine's active studio
    /// profile.
    ///
    /// `None` when the project is not part of an album, when the album
    /// file does not parse, or when the list double-books a role the
    /// profile does not share — each of which is one `warn` line and a
    /// window with no Patch List in it, never a window that will not
    /// open. A machine with no profile is NOT one of those: the table
    /// builds against an empty profile and every role shows unresolved,
    /// which is exactly what a list written in another room should look
    /// like here.
    // r[impl flow.patch-list.project-level]
    // r[impl flow.patch-list.studio-profiles]
    #[must_use]
    pub fn for_project(project: &std::path::Path) -> Option<Self> {
        let dir = project.parent()?;
        let album = patch_list::find_album(dir)?;
        let text = std::fs::read_to_string(&album)
            .inspect_err(|error| {
                tracing::warn!(error = %error, "the album's patch list did not read");
            })
            .ok()?;
        let list = PatchList::from_styx(&text)
            .inspect_err(|error| {
                tracing::warn!(error = %error, "the album's patch list did not parse");
            })
            .ok()?;
        let (studio, profile) = patch_list::Studios::in_config_dir()
            .and_then(|studios| studios.active())
            .unwrap_or_else(|| ("none".to_owned(), StudioProfile::default()));
        Self::build(&studio, &list, &profile)
            .inspect_err(|error| {
                tracing::warn!(error = %error, "the album's patch list is not valid");
            })
            .ok()
    }

    /// The fixture album in the fixture room — what the render fixture
    /// is a picture of.
    ///
    /// # Errors
    ///
    /// When the committed fixtures stop parsing or stop validating,
    /// which means a broken fixture rather than a broken machine — so
    /// it is returned and named rather than panicked, and the test that
    /// reads it says which file to look at.
    pub fn fixture() -> Result<Self, String> {
        let list = PatchList::from_styx(FIXTURE_ALBUM)
            .map_err(|e| format!("the fixture album does not parse: {e}"))?;
        let room = StudioProfile::from_styx(FIXTURE_ROOM)
            .map_err(|e| format!("the fixture room does not parse: {e}"))?;
        Self::build("golden-room", &list, &room)
            .map_err(|e| format!("the fixture album is not valid: {e}"))
    }
}

/// How a resolution reads in the Input column.
fn describe(resolved: &Resolved) -> String {
    match resolved {
        // One-based for a human: the profile counts channels from zero
        // because the daw's `RecordInput` does, and nobody patching a
        // room counts that way.
        Resolved::Audio { channel } => format!("ch {}", channel.saturating_add(1)),
        Resolved::Midi { device, channel } => channel.as_ref().map_or_else(
            || format!("MIDI {device} all ch"),
            |channel| format!("MIDI {device} ch {channel}"),
        ),
        Resolved::Pair { left, right } => {
            format!("out {}/{}", left.saturating_add(1), right.saturating_add(1))
        }
        Resolved::Unresolved => "unresolved".to_owned(),
    }
}

const fn mark(resolved: &Resolved) -> Mark {
    match resolved {
        Resolved::Unresolved => Mark::Unresolved,
        _ => Mark::Resolved,
    }
}

/// The row height.
const ROW_H: f64 = 20.0;
/// A performer header's height.
const HEADER_H: f64 = 28.0;
/// The gutter down each side.
const PAD: f64 = 16.0;
/// Where each column starts, from the left edge of the table.
const COL_KIND: f64 = 24.0;
const COL_KEY: f64 = 132.0;
const COL_ROLE: f64 = 340.0;
const COL_INPUT: f64 = 560.0;
/// The type sizes.
const TITLE_SIZE: f32 = 17.0;
const HEADER_SIZE: f32 = 14.0;
const ROW_SIZE: f32 = 12.5;

/// Draw the table into a scene, top-left at `origin`, `width` wide.
///
/// Returns the height it drew, so a caller can scroll it.
// r[impl flow.patch-list.plan]
pub fn paint(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    table: &Table,
    origin: (f64, f64),
    width: f64,
) -> f64 {
    let (x0, y0) = origin;
    let mut y = heading(painter, palette, font, table, origin);
    y = columns(
        painter,
        palette,
        font,
        ["Source", "Mic", "Role", "Channel"],
        x0,
        y,
        width,
    );
    y = performers(painter, palette, font, table, (x0, y), width);
    y += 12.0;
    y = columns(
        painter,
        palette,
        font,
        ["Headphones", "For", "Role", "Output"],
        x0,
        y,
        width,
    );
    y = buses(painter, palette, font, table, (x0, y), width);
    y - y0 + PAD
}

/// The title and what the room made of the list, returning the y under
/// them.
fn heading(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    table: &Table,
    origin: (f64, f64),
) -> f64 {
    let (x0, y0) = origin;
    let y = y0 + PAD;
    let baseline = y + f64::from(TITLE_SIZE);
    crate::tcp::glyphs(
        painter,
        font,
        palette.text,
        "Patch List",
        x0 + PAD,
        baseline,
        TITLE_SIZE,
    );
    // The count is the whole point of the summary, so it is coloured
    // like the rows it counts: nothing unresolved reads as chrome, one
    // unresolved role reads as the thing to look at.
    let (summary, color) = if table.unresolved == 0 {
        (
            format!("{} — every role resolved", table.studio),
            palette.text_dim,
        )
    } else {
        (
            format!("{} — {} unresolved", table.studio, table.unresolved),
            palette.rec,
        )
    };
    crate::tcp::glyphs(
        painter,
        font,
        color,
        &summary,
        x0 + PAD + 120.0,
        baseline,
        ROW_SIZE,
    );
    y + f64::from(TITLE_SIZE) + 12.0
}

/// Every performer's header and rig, returning the y under them.
fn performers(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    table: &Table,
    origin: (f64, f64),
    width: f64,
) -> f64 {
    let (x0, mut y) = origin;
    // One running count across the whole table, so the banding does not
    // restart at each performer and make two adjacent rigs look like
    // one block.
    let mut striped = 0_usize;
    for performer in &table.performers {
        fill(
            painter,
            palette.tcp_column,
            Rect::new(x0 + PAD, y, x0 + width - PAD, y + HEADER_H),
        );
        crate::tcp::glyphs(
            painter,
            font,
            palette.text,
            &performer.name,
            x0 + PAD + 8.0,
            y + HEADER_H - 9.0,
            HEADER_SIZE,
        );
        y += HEADER_H;
        for row in &performer.rows {
            line(
                painter,
                palette,
                font,
                &Line {
                    cells: [&row.kind, &row.key, &row.role, &row.input],
                    mark: row.mark,
                    striped: striped.is_multiple_of(2),
                },
                (x0, y),
                width,
            );
            striped = striped.wrapping_add(1);
            y += ROW_H;
        }
    }
    y
}

/// The headphone buses, returning the y under them.
fn buses(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    table: &Table,
    origin: (f64, f64),
    width: f64,
) -> f64 {
    let (x0, mut y) = origin;
    for (index, bus) in table.buses.iter().enumerate() {
        line(
            painter,
            palette,
            font,
            &Line {
                cells: [&bus.name, &bus.audience, &bus.role, &bus.input],
                mark: bus.mark,
                striped: index.is_multiple_of(2),
            },
            (x0, y),
            width,
        );
        y += ROW_H;
    }
    y
}

/// One row of the table, whichever half it is in.
struct Line<'a> {
    /// The four columns, left to right.
    cells: [&'a str; 4],
    /// Whether the room resolved it — the last column's colour.
    mark: Mark,
    /// Which band this row takes.
    striped: bool,
}

/// Draw one row: its band, then its four cells.
fn line(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    row: &Line<'_>,
    origin: (f64, f64),
    width: f64,
) {
    let (x0, y) = origin;
    let band = if row.striped {
        palette.row_a
    } else {
        palette.row_b
    };
    fill(
        painter,
        band,
        Rect::new(x0 + PAD, y, x0 + width - PAD, y + ROW_H),
    );
    let baseline = y + ROW_H - 6.0;
    let last = match row.mark {
        Mark::Resolved => palette.text,
        Mark::Unresolved => palette.rec,
    };
    // The first column names the thing, the middle two qualify it, the
    // last is the answer — so the middle two are dimmer than either end.
    let colors = [palette.text_dim, palette.text, palette.text_dim, last];
    for ((body, color), x) in row
        .cells
        .iter()
        .zip(colors)
        .zip([COL_KIND, COL_KEY, COL_ROLE, COL_INPUT])
    {
        cell(painter, font, color, body, x0 + x, baseline);
    }
}

/// A column header strip, returning the y under it.
fn columns(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    names: [&str; 4],
    x0: f64,
    y: f64,
    width: f64,
) -> f64 {
    let baseline = y + ROW_H - 6.0;
    for (name, x) in names
        .into_iter()
        .zip([COL_KIND, COL_KEY, COL_ROLE, COL_INPUT])
    {
        cell(painter, font, palette.text_faint, name, x0 + x, baseline);
    }
    fill(
        painter,
        palette.tcp_rule,
        Rect::new(x0 + PAD, y + ROW_H, x0 + width - PAD, y + ROW_H + 1.0),
    );
    y + ROW_H + 1.0
}

fn cell(
    painter: &mut impl PaintScene,
    font: &Font,
    color: Color,
    body: &str,
    x: f64,
    baseline: f64,
) {
    crate::tcp::glyphs(painter, font, color, body, x, baseline, ROW_SIZE);
}

fn fill(painter: &mut impl PaintScene, color: Color, rect: Rect) {
    painter.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
}

/// Render the view headlessly into RGBA8 bytes.
///
/// `None` when this box has no usable GPU — the picture is a fixture,
/// and a software fallback would render a different one.
#[must_use]
pub fn shot(table: &Table, size: (u32, u32)) -> Option<Vec<u8>> {
    use anyrender::ImageRenderer;

    let (width, height) = size;
    let theme = daw_ui::theming::Theme::dark();
    let palette = Palette::from_theme(&theme);
    let font = Font::embedded().ok()?;
    let mut renderer = anyrender_vello::VelloImageRenderer::new(width, height);
    let mut buffer = Vec::new();
    renderer.render_to_vec(
        |painter| {
            painter.reset();
            painter.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.surface,
                None,
                &Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
            );
            paint(
                painter,
                &palette,
                &font,
                table,
                (0.0, 0.0),
                f64::from(width),
            );
        },
        &mut buffer,
    );
    Some(buffer)
}
