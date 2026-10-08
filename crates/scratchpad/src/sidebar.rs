//! Note navigation: the search field, the "New Note" button and the note list grouped by date
//! or, while searching, the matching notes (PLAN §8, §28, §42-43). See ADR 0051.

use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Local, NaiveDate};
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, Entity, FocusHandle, Focusable,
    FontWeight, HighlightStyle, Pixels, ScrollStrategy, Stateful, StyledText, Subscription,
    UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use scratchpad_core::{DateGroup, Note, local_date};

use crate::notes::{Notes, Selection};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::{ActiveTheme, Theme, typography};

pub const DEFAULT_SIDEBAR_WIDTH: Pixels = px(260.);

/// Every list row has the same height so the list can be virtualized with `uniform_list`.
/// Section headers put their label at the bottom, so the space above it separates sections.
const ROW_HEIGHT: Pixels = px(46.);

/// Segoe MDL2 Assets glyphs; plain text elsewhere.
const ADD_ICON: &str = if cfg!(windows) { "\u{E710}" } else { "+" };
const SEARCH_ICON: &str = if cfg!(windows) { "\u{E721}" } else { "" };

/// One line of the note list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Header(DateGroup),
    Draft,
    /// Index into [`Notes::notes`].
    Note(usize),
    /// Index into [`Notes::search_hits`].
    Hit(usize),
}

pub struct Sidebar {
    notes: Entity<Notes>,
    width: Pixels,
    search: Entity<TextInput>,
    /// Where Escape in the search field returns focus to.
    focus_before_search: Option<FocusHandle>,
    list_focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// The list rows, rebuilt only when the notes change or the day does (the groups are
    /// relative to today), so frames (hover, caret, scrolling) cost the same with 10 or
    /// 10,000 notes.
    rows: Rc<[Row]>,
    rows_date: NaiveDate,
    _subscriptions: [Subscription; 2],
}

impl Sidebar {
    pub fn new(notes: Entity<Notes>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new("Search", cx));
        let subscriptions = [
            cx.observe(&notes, |this, notes, cx| {
                // The query can also be cleared by the model (e.g. by a new note).
                let query = notes.read(cx).query().to_owned();
                this.search
                    .update(cx, |search, cx| search.set_text(&query, cx));
                this.rebuild_rows(cx);
                cx.notify();
            }),
            cx.subscribe_in(&search, window, Self::on_search_event),
        ];
        let mut sidebar = Self {
            notes,
            width: DEFAULT_SIDEBAR_WIDTH,
            search,
            focus_before_search: None,
            list_focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            rows: Rc::new([]),
            rows_date: local_date(SystemTime::now()),
            _subscriptions: subscriptions,
        };
        sidebar.rebuild_rows(cx);
        sidebar
    }

    fn rebuild_rows(&mut self, cx: &App) {
        self.rows_date = local_date(SystemTime::now());
        self.rows = rows(self.notes.read(cx), self.rows_date).into();
    }

    /// The text in the search field.
    pub fn search_text(&self, cx: &App) -> String {
        self.search.read(cx).text().to_owned()
    }

    /// Moves focus to the search field and selects its text (Ctrl+P, Ctrl+Shift+F).
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search = self.search.focus_handle(cx);
        if !search.is_focused(window) {
            self.focus_before_search = window.focused(cx);
        }
        window.focus(&search);
        self.search.update(cx, |search, cx| search.select_all(cx));
    }

    fn on_search_event(
        &mut self,
        search: &Entity<TextInput>,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextInputEvent::Changed => {
                let query = search.read(cx).text().to_owned();
                self.notes
                    .update(cx, |notes, cx| notes.set_query(&query, cx));
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            TextInputEvent::Confirmed => {
                let first_hit = self
                    .notes
                    .read(cx)
                    .search_hits()
                    .and_then(|hits| hits.first().map(|hit| hit.path.clone()));
                if let Some(path) = first_hit {
                    self.open(path, window, cx);
                }
            }
            TextInputEvent::Cancelled => {
                search.update(cx, |search, cx| search.set_text("", cx));
                match self.focus_before_search.take() {
                    Some(previous) => window.focus(&previous),
                    None => window.focus(&self.list_focus),
                }
            }
        }
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
            Row::Hit(ix) => {
                let notes = self.notes.read(cx);
                let Some(hit) = notes.search_hits().and_then(|hits| hits.get(ix)) else {
                    return div().into_any_element();
                };
                let path = hit.path.clone();
                let selected = matches!(notes.selection(), Selection::Note(open) if *open == path);
                let title = highlighted(&hit.title, &hit.title_ranges, theme);
                let subtitle = if hit.snippet.is_empty() {
                    let today = self.rows_date;
                    let note = notes.note(&hit.path);
                    StyledText::new(
                        note.map(|note| modified_label(note, today))
                            .unwrap_or_default(),
                    )
                } else {
                    highlighted(&hit.snippet, &hit.snippet_ranges, theme)
                };
                self.note_row(("hit", ix), title, subtitle, selected, window, cx)
                    .debug_selector(|| format!("hit:{}", hit.title))
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
        title: impl IntoElement,
        subtitle: impl IntoElement,
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
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted)
                        .child(subtitle),
                ),
        )
    }
}

fn rows(notes: &Notes, today: NaiveDate) -> Vec<Row> {
    if notes.is_searching() {
        let hits = notes.search_hits().map_or(0, |hits| hits.len());
        return (0..hits).map(Row::Hit).collect();
    }
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

/// `text` with the matched `ranges` marked like selected text.
fn highlighted(text: &str, ranges: &[Range<usize>], theme: &Theme) -> StyledText {
    let style = HighlightStyle {
        color: Some(theme.foreground),
        background_color: Some(theme.selection),
        ..Default::default()
    };
    StyledText::new(text.to_owned())
        .with_highlights(ranges.iter().map(|range| (range.clone(), style)))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if local_date(SystemTime::now()) != self.rows_date {
            self.rebuild_rows(cx);
        }
        let theme = cx.theme().clone();
        let notes = self.notes.read(cx);
        let rows = self.rows.clone();
        let empty_text = match (notes.is_searching(), notes.search_hits()) {
            (true, Some([])) => Some("No results"),
            (false, _) if notes.is_loaded() && rows.is_empty() => Some("No notes"),
            _ => None,
        };
        let search_focused = self.search.focus_handle(cx).is_focused(window);

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
                    .id("search-field")
                    .debug_selector(|| "search-field".into())
                    .mx_3()
                    .mt_3()
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .bg(theme.background)
                    .border_1()
                    .border_color(if search_focused {
                        theme.accent
                    } else {
                        theme.border
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        window.focus(&this.search.focus_handle(cx))
                    }))
                    .child(
                        div()
                            .font_family(typography::ICON_FONT_FAMILY)
                            .text_xs()
                            .text_color(theme.muted)
                            .child(SEARCH_ICON),
                    )
                    .child(div().flex_1().min_w_0().child(self.search.clone())),
            )
            .child(
                div()
                    .id("new-note")
                    .debug_selector(|| "new-note-button".into())
                    .mx_2()
                    .mt_2()
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
                    .when_some(empty_text, |list, text| {
                        list.flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme.muted)
                            .child(
                                div()
                                    .debug_selector(|| text.to_lowercase().replace(' ', "-"))
                                    .child(text),
                            )
                    })
                    .when(empty_text.is_none(), |list| {
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
