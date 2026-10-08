//! Note navigation: the "New Note" button and the note list grouped by date (PLAN §8, §42-43).
//! See ADR 0051.

use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Local, NaiveDate};
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, Entity, FocusHandle, FontWeight, Pixels,
    ScrollStrategy, SharedString, Stateful, Subscription, UniformListScrollHandle, Window, div,
    prelude::*, px, uniform_list,
};
use scratchpad_core::{DateGroup, Note, local_date};

use crate::notes::{Notes, Selection};
use crate::theme::{ActiveTheme, typography};

pub const DEFAULT_SIDEBAR_WIDTH: Pixels = px(260.);

/// Every list row has the same height so the list can be virtualized with `uniform_list`.
/// Section headers put their label at the bottom, so the space above it separates sections.
const ROW_HEIGHT: Pixels = px(46.);

/// Segoe MDL2 Assets glyphs; plain text elsewhere.
const ADD_ICON: &str = if cfg!(windows) { "\u{E710}" } else { "+" };

/// One line of the note list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Header(DateGroup),
    Draft,
    /// Index into [`Notes::notes`].
    Note(usize),
}

pub struct Sidebar {
    notes: Entity<Notes>,
    width: Pixels,
    list_focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// The list rows, rebuilt only when the notes change or the day does (the groups are
    /// relative to today), so frames (hover, caret, scrolling) cost the same with 10 or
    /// 10,000 notes.
    rows: Rc<[Row]>,
    rows_date: NaiveDate,
    _observe_notes: Subscription,
}

impl Sidebar {
    pub fn new(notes: Entity<Notes>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut sidebar = Self {
            _observe_notes: cx.observe(&notes, |this, _, cx| {
                this.rebuild_rows(cx);
                cx.notify();
            }),
            notes,
            width: DEFAULT_SIDEBAR_WIDTH,
            list_focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            rows: Rc::new([]),
            rows_date: local_date(SystemTime::now()),
        };
        sidebar.rebuild_rows(cx);
        sidebar
    }

    fn rebuild_rows(&mut self, cx: &App) {
        self.rows_date = local_date(SystemTime::now());
        self.rows = rows(self.notes.read(cx), self.rows_date).into();
    }

    /// Starts a new note and shows it at the top of the list.
    pub fn new_note(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.notes.update(cx, |notes, cx| notes.new_note(cx));
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.list_focus);
        self.notes.update(cx, |notes, cx| notes.select(&path, cx));
    }

    fn render_row(&self, row: Row, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        match row {
            Row::Header(group) => div()
                .debug_selector(|| format!("group:{}", group.label()))
                .h(ROW_HEIGHT)
                .px_5()
                .pb_1()
                .flex()
                .items_end()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted)
                .child(group.label())
                .into_any_element(),
            // The draft only exists while it is open, so it is always selected.
            Row::Draft => self
                .note_row("draft", "New Note", "No additional text", true, window, cx)
                .debug_selector(|| "note:draft".into())
                .into_any_element(),
            Row::Note(ix) => {
                let notes = self.notes.read(cx);
                let Some(note) = notes.notes().get(ix) else {
                    return div().into_any_element();
                };
                let path = note.path.clone();
                let selected = matches!(notes.selection(), Selection::Note(open) if *open == path);
                let subtitle = modified_label(note, self.rows_date);
                self.note_row(ix, note.title.clone(), subtitle, selected, window, cx)
                    .debug_selector(|| format!("note:{}", note.title))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open(path.clone(), window, cx)
                    }))
                    .into_any_element()
            }
        }
    }

    /// A two-line row: the title and a muted second line.
    fn note_row(
        &self,
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
        selected: bool,
        window: &Window,
        cx: &App,
    ) -> Stateful<Div> {
        let theme = cx.theme();
        // Accent while the list has focus, neutral otherwise, as in native lists.
        let selection = if self.list_focus.contains_focused(window, cx) {
            theme.selection
        } else {
            theme.foreground.opacity(0.08)
        };
        div().id(id).h(ROW_HEIGHT).px_2().child(
            div()
                .size_full()
                .px_3()
                .flex()
                .flex_col()
                .justify_center()
                .rounded_md()
                .map(|row| {
                    if selected {
                        row.bg(selection)
                    } else {
                        row.hover(|style| style.bg(theme.foreground.opacity(0.04)))
                    }
                })
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(title.into()),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted)
                        .child(subtitle.into()),
                ),
        )
    }
}

fn rows(notes: &Notes, today: NaiveDate) -> Vec<Row> {
    let mut rows = Vec::with_capacity(notes.notes().len() + 6);
    let mut current = None;
    if notes.has_draft() {
        rows.extend([Row::Header(DateGroup::Today), Row::Draft]);
        current = Some(DateGroup::Today);
    }
    for (ix, note) in notes.notes().iter().enumerate() {
        let group = DateGroup::of(local_date(note.modified_at), today);
        if current != Some(group) {
            rows.push(Row::Header(group));
            current = Some(group);
        }
        rows.push(Row::Note(ix));
    }
    rows
}

/// Time for today and yesterday, weekday within a week, date otherwise.
fn modified_label(note: &Note, today: NaiveDate) -> String {
    let Some(modified) = local_time(note.modified_at) else {
        return String::new();
    };
    match DateGroup::of(modified.date_naive(), today) {
        DateGroup::Today | DateGroup::Yesterday => modified.format("%H:%M").to_string(),
        DateGroup::Previous7Days => modified.format("%A").to_string(),
        _ if modified.year() == today.year() => modified.format("%-d %B").to_string(),
        _ => modified.format("%-d %B %Y").to_string(),
    }
}

/// `None` for timestamps chrono cannot represent (corrupt file metadata).
fn local_time(time: SystemTime) -> Option<DateTime<Local>> {
    let seconds = time.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let utc = DateTime::from_timestamp(i64::try_from(seconds).ok()?, 0)?;
    Some(utc.with_timezone(&Local))
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if local_date(SystemTime::now()) != self.rows_date {
            self.rebuild_rows(cx);
        }
        let theme = cx.theme().clone();
        let notes = self.notes.read(cx);
        let rows = self.rows.clone();
        let empty = notes.is_loaded() && rows.is_empty();

        div()
            .debug_selector(|| "sidebar".into())
            .flex_none()
            .w(self.width)
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .border_r_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("new-note")
                    .debug_selector(|| "new-note-button".into())
                    .mx_2()
                    .mt_3()
                    .mb_1()
                    .px_3()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.foreground.opacity(0.05)))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.new_note(window, cx)),
                    )
                    .child(
                        div()
                            .font_family(typography::ICON_FONT_FAMILY)
                            .text_xs()
                            .text_color(theme.accent)
                            .child(ADD_ICON),
                    )
                    .child("New Note"),
            )
            .child(
                div()
                    .id("note-list")
                    .track_focus(&self.list_focus)
                    .key_context("NoteList")
                    .flex_1()
                    .min_h_0()
                    .when(empty, |list| {
                        list.flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme.muted)
                            .child("No notes")
                    })
                    .when(!empty, |list| {
                        list.child(
                            uniform_list(
                                "notes",
                                rows.len(),
                                cx.processor(move |this, range: Range<usize>, window, cx| {
                                    range
                                        .map(|ix| this.render_row(rows[ix], window, cx))
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .size_full()
                            .pb_2()
                            .track_scroll(self.scroll.clone()),
                        )
                    }),
            )
    }
}
