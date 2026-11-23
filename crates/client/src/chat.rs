//! Chat, the way Minecraft has it: T opens it, / opens it with a slash typed. Enter sends (a
//! message, or a command if it starts with /), Esc closes. Up and Down bring back what you sent;
//! while typing a command its suggestions show above the line (Tab takes the highlighted one and
//! cycles, Up and Down move through them) with the rest of it in gray after the cursor. Messages
//! show at the bottom left for ten seconds, fading out over the last; with the chat open the last
//! twenty show, and the wheel or Page Up/Down scrolls back.
//!
//! The action bar (`ActionBar`) is the short message over the middle of the screen that blocks
//! and stations answer with ("The station can't grow further").
//!
//! Keyboard focus: while the chat (or another text field) has it, `Keyboard::owner` says who,
//! the keys go only there (`ButtonInput<KeyCode>` is emptied each frame, so nothing else moves),
//! and the mouse is let go.

use crate::commands::{self, Suggestions};
use crate::map::GalaxyMap;
use crate::menu::PlanetMenu;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

/// Seconds a message stays when the chat is closed; it fades over the last one.
const SHOW: f32 = 10.0;
/// Lines shown closed and open, and kept.
const CLOSED_LINES: usize = 10;
const OPEN_LINES: usize = 20;
const KEPT: usize = 100;
/// What you sent, kept for Up and Down.
const SENT_KEPT: usize = 100;
const FONT: f32 = 16.0;
/// The default font is monospaced: every character this wide.
const CHAR_W: f32 = FONT * 0.6;
const LINE_H: f32 = 20.0;
const WIDTH: f32 = 560.0;
/// The longest line you can type, as in Minecraft.
const MAX_INPUT: usize = 256;

pub const WHITE: Color = Color::WHITE;
pub const GRAY: Color = Color::srgb(0.667, 0.667, 0.667);
pub const DARK_GRAY: Color = Color::srgb(0.333, 0.333, 0.333);
pub const RED: Color = Color::srgb(1.0, 0.333, 0.333);
pub const GREEN: Color = Color::srgb(0.333, 1.0, 0.333);
pub const YELLOW: Color = Color::srgb(1.0, 1.0, 0.333);
pub const AQUA: Color = Color::srgb(0.333, 1.0, 1.0);
const BACK: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);
const SUGGEST_BACK: Color = Color::srgba(0.0, 0.0, 0.0, 0.82);

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Keyboard>()
            .init_resource::<Chat>()
            .init_resource::<ActionBar>()
            .add_systems(Startup, spawn_ui)
            .add_systems(PreUpdate, capture_keys.after(InputSystems))
            .add_systems(
                Update,
                (chat_keys, commands::run, draw_chat, draw_action_bar)
                    .chain()
                    .after(crate::player::sync_camera),
            );
    }
}

/// Who the keyboard is typing into, if anyone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    Chat,
    StationMenu,
    Schematics,
    Inventory,
    Dialogue,
}

#[derive(Resource, Default)]
pub struct Keyboard {
    pub owner: Option<Owner>,
    /// This frame's key presses, for the owner.
    pub events: Vec<KeyboardInput>,
    /// Ctrl is held (kept from the key events: the key state is emptied while typing).
    pub ctrl: bool,
}

/// What a key did to a text field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    None,
    Changed,
    Moved,
    Enter,
    Escape,
    Tab,
    Up,
    Down,
    PageUp,
    PageDown,
}

/// One line of text being typed, with a cursor.
#[derive(Clone, Debug, Default)]
pub struct TextField {
    pub text: String,
    /// In characters.
    pub cursor: usize,
    max: usize,
    blink: f32,
}

impl TextField {
    pub fn new(max: usize) -> Self {
        Self { max, ..default() }
    }

    pub fn set(&mut self, s: &str) {
        self.text = s.chars().take(self.max).collect();
        self.cursor = self.text.chars().count();
        self.blink = 0.0;
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map_or(self.text.len(), |(b, _)| b)
    }

    /// The text up to the cursor.
    pub fn before(&self) -> &str {
        &self.text[..self.byte(self.cursor)]
    }

    pub fn after(&self) -> &str {
        &self.text[self.byte(self.cursor)..]
    }

    /// Where the word before (dir -1) or after (+1) the cursor starts, Ctrl+arrows style.
    fn word(&self, dir: i32) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        if dir < 0 {
            while i > 0 && chars[i - 1] == ' ' {
                i -= 1;
            }
            while i > 0 && chars[i - 1] != ' ' {
                i -= 1;
            }
        } else {
            while i < chars.len() && chars[i] != ' ' {
                i += 1;
            }
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
        }
        i
    }

    fn remove(&mut self, from: usize, to: usize) {
        let (a, b) = (self.byte(from), self.byte(to));
        self.text.replace_range(a..b, "");
        self.cursor = from;
    }

    /// Applies a key press.
    pub fn apply(&mut self, ev: &KeyboardInput, ctrl: bool) -> Edit {
        if !ev.state.is_pressed() {
            return Edit::None;
        }
        self.blink = 0.0;
        let n = self.len();
        match &ev.logical_key {
            Key::Enter => Edit::Enter,
            Key::Escape => Edit::Escape,
            Key::Tab => Edit::Tab,
            Key::ArrowUp => Edit::Up,
            Key::ArrowDown => Edit::Down,
            Key::PageUp => Edit::PageUp,
            Key::PageDown => Edit::PageDown,
            Key::ArrowLeft => {
                self.cursor = if ctrl {
                    self.word(-1)
                } else {
                    self.cursor.saturating_sub(1)
                };
                Edit::Moved
            }
            Key::ArrowRight => {
                self.cursor = if ctrl {
                    self.word(1)
                } else {
                    (self.cursor + 1).min(n)
                };
                Edit::Moved
            }
            Key::Home => {
                self.cursor = 0;
                Edit::Moved
            }
            Key::End => {
                self.cursor = n;
                Edit::Moved
            }
            Key::Backspace if self.cursor > 0 => {
                let from = if ctrl { self.word(-1) } else { self.cursor - 1 };
                self.remove(from, self.cursor);
                Edit::Changed
            }
            Key::Delete if self.cursor < n => {
                let to = if ctrl { self.word(1) } else { self.cursor + 1 };
                let c = self.cursor;
                self.remove(c, to);
                Edit::Changed
            }
            _ => {
                let Some(t) = ev.text.as_ref() else {
                    return Edit::None;
                };
                if ctrl {
                    return Edit::None;
                }
                let add: String = t
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(self.max.saturating_sub(n))
                    .collect();
                if add.is_empty() {
                    return Edit::None;
                }
                let at = self.byte(self.cursor);
                self.text.insert_str(at, &add);
                self.cursor += add.chars().count();
                Edit::Changed
            }
        }
    }

    /// Advances the cursor's blink.
    pub fn tick(&mut self, dt: f32) {
        self.blink += dt;
    }

    /// The cursor shows for 0.3 s, hides for 0.3 s.
    pub fn cursor_on(&self) -> bool {
        ((self.blink / 0.3) as u32).is_multiple_of(2)
    }

    /// The text with its blinking cursor: `_` at the end, `|` inside it.
    pub fn shown(&self) -> String {
        let c = if !self.cursor_on() {
            ""
        } else if self.cursor == self.len() {
            "_"
        } else {
            "|"
        };
        format!("{}{}{}", self.before(), c, self.after())
    }
}

/// One line of the chat, in colored pieces.
#[derive(Clone, Debug)]
pub struct Line {
    pub spans: Vec<(String, Color)>,
    /// When it came, seconds.
    at: f32,
}

#[derive(Resource)]
pub struct Chat {
    pub open: bool,
    pub field: TextField,
    /// Lines, oldest first.
    lines: Vec<Line>,
    /// Lines scrolled back from the newest.
    scroll: usize,
    /// What you sent, oldest first, and where Up/Down are in it (None: the line being typed).
    sent: Vec<String>,
    recall: Option<usize>,
    draft: String,
    pub suggest: Suggestions,
    /// Lines sent, waiting for `commands::run`.
    pub pending: Vec<String>,
    regrab: bool,
    now: f32,
    /// Something to show changed.
    dirty: bool,
}

impl Default for Chat {
    fn default() -> Self {
        Self {
            open: false,
            field: TextField::new(MAX_INPUT),
            lines: Vec::new(),
            scroll: 0,
            sent: Vec::new(),
            recall: None,
            draft: String::new(),
            suggest: Suggestions::default(),
            pending: Vec::new(),
            regrab: false,
            now: 0.0,
            dirty: true,
        }
    }
}

impl Chat {
    /// Adds a line of colored pieces.
    pub fn line(&mut self, spans: Vec<(String, Color)>) {
        // Long lines wrap, as in Minecraft.
        let per_line = (WIDTH / CHAR_W) as usize - 1;
        let mut cur: Vec<(String, Color)> = Vec::new();
        let mut used = 0;
        for (text, color) in spans {
            let mut rest: &str = &text;
            while !rest.is_empty() {
                let room = per_line - used;
                let n = rest.chars().count();
                if n <= room {
                    cur.push((rest.to_string(), color));
                    used += n;
                    break;
                }
                // Break at the last space that fits, else mid-word.
                let cut_chars = rest.chars().take(room).collect::<String>();
                let cut = cut_chars
                    .rfind(' ')
                    .filter(|&b| b > 0)
                    .map_or(cut_chars.len(), |b| b + 1);
                cur.push((rest[..cut].to_string(), color));
                self.push(std::mem::take(&mut cur));
                used = 0;
                rest = &rest[cut..];
            }
        }
        if !cur.is_empty() {
            self.push(cur);
        }
    }

    fn push(&mut self, spans: Vec<(String, Color)>) {
        info!(
            "[CHAT] {}",
            spans.iter().map(|s| s.0.as_str()).collect::<String>()
        );
        self.lines.push(Line {
            spans,
            at: self.now,
        });
        if self.lines.len() > KEPT {
            self.lines.remove(0);
        }
        if self.scroll > 0 {
            self.scroll += 1;
        }
        self.dirty = true;
    }

    /// A line in one color; several lines if it has line breaks.
    pub fn say(&mut self, text: &str, color: Color) {
        for l in text.lines() {
            self.line(vec![(l.to_string(), color)]);
        }
    }

    pub fn info(&mut self, text: &str) {
        self.say(text, WHITE);
    }

    pub fn error(&mut self, text: &str) {
        self.say(text, RED);
    }

    fn close(&mut self, keyboard: &mut Keyboard, cursor: &mut CursorOptions) {
        self.open = false;
        self.scroll = 0;
        self.recall = None;
        self.suggest = Suggestions::default();
        self.field.set("");
        keyboard.owner = None;
        if self.regrab {
            cursor.grab_mode = CursorGrabMode::Locked;
            cursor.visible = false;
        }
        self.dirty = true;
    }
}

/// The short message over the middle of the screen.
#[derive(Resource, Default)]
pub struct ActionBar {
    text: String,
    left: f32,
}

impl ActionBar {
    /// What it says now (kept after it fades).
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn show(&mut self, text: &str) {
        self.text = text.into();
        self.left = 3.0;
    }
}

/// Keys for whoever has the keyboard; T or / opens the chat when nobody does.
#[allow(clippy::too_many_arguments)]
fn capture_keys(
    mut events: MessageReader<KeyboardInput>,
    mut keyboard: ResMut<Keyboard>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut chat: ResMut<Chat>,
    menu: Res<PlanetMenu>,
    map: Res<GalaxyMap>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    keyboard.events.clear();
    for ev in events.read() {
        if matches!(ev.key_code, KeyCode::ControlLeft | KeyCode::ControlRight) {
            keyboard.ctrl = ev.state.is_pressed();
        }
        if keyboard.owner.is_some() {
            keyboard.events.push(ev.clone());
            continue;
        }
        if !ev.state.is_pressed() || ev.repeat || menu.is_open() || map.is_open() || keyboard.ctrl {
            continue;
        }
        let opener = match &ev.logical_key {
            Key::Character(c) if c.eq_ignore_ascii_case("t") => Some(""),
            Key::Character(c) if c.as_str() == "/" => Some("/"),
            _ => None,
        };
        if let Some(start) = opener {
            chat.open = true;
            chat.field.set(start);
            chat.recall = None;
            chat.scroll = 0;
            chat.dirty = true;
            chat.regrab = cursor.grab_mode != CursorGrabMode::None;
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
            keyboard.owner = Some(Owner::Chat);
            // The rest of this frame's keys are dropped: the opening key's own letter among them.
            break;
        }
    }
    events.read().for_each(drop);
    if keyboard.owner.is_some() {
        keys.reset_all();
    }
}

#[allow(clippy::too_many_arguments)]
fn chat_keys(
    mut chat: ResMut<Chat>,
    mut keyboard: ResMut<Keyboard>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    ctx: commands::Context,
) {
    chat.now = time.elapsed_secs();
    if !chat.open || keyboard.owner != Some(Owner::Chat) {
        return;
    }
    chat.field.tick(time.delta_secs());
    if scroll.delta.y != 0.0 {
        let max = chat.lines.len().saturating_sub(OPEN_LINES);
        chat.scroll =
            (chat.scroll as i32 + scroll.delta.y.signum() as i32).clamp(0, max as i32) as usize;
        chat.dirty = true;
    }
    let ctrl = keyboard.ctrl;
    let events = std::mem::take(&mut keyboard.events);
    for ev in &events {
        let edit = chat.field.apply(ev, ctrl);
        let chat = &mut *chat;
        match edit {
            Edit::None => continue,
            Edit::Escape => {
                if chat.suggest.visible() {
                    chat.suggest.hide();
                } else {
                    chat.close(&mut keyboard, &mut cursor);
                    return;
                }
            }
            Edit::Enter => {
                let line = chat.field.text.trim().to_string();
                if !line.is_empty() {
                    if chat.sent.last() != Some(&line) {
                        chat.sent.push(line.clone());
                        if chat.sent.len() > SENT_KEPT {
                            chat.sent.remove(0);
                        }
                    }
                    chat.pending.push(line);
                }
                chat.close(&mut keyboard, &mut cursor);
                return;
            }
            Edit::Tab => {
                if let Some(text) = chat.suggest.complete(&chat.field.text) {
                    chat.field.set(&text);
                    chat.suggest.keep_after_tab();
                } else {
                    chat.suggest
                        .refresh(&chat.field.text, chat.field.before(), &ctx);
                }
            }
            Edit::Up | Edit::Down if chat.suggest.visible() => {
                chat.suggest.select(if edit == Edit::Up { -1 } else { 1 })
            }
            Edit::Up | Edit::Down => {
                // Back and forth through what you sent.
                let n = chat.sent.len();
                if n == 0 {
                    continue;
                }
                let next = match (chat.recall, edit) {
                    (None, Edit::Up) => {
                        chat.draft = chat.field.text.clone();
                        Some(n - 1)
                    }
                    (Some(i), Edit::Up) => Some(i.saturating_sub(1)),
                    (Some(i), _) if i + 1 < n => Some(i + 1),
                    _ => None,
                };
                chat.recall = next;
                let text = next.map_or(chat.draft.clone(), |i| chat.sent[i].clone());
                chat.field.set(&text);
                chat.suggest
                    .refresh(&chat.field.text, chat.field.before(), &ctx);
                chat.suggest.hide();
            }
            Edit::PageUp | Edit::PageDown => {
                let max = chat.lines.len().saturating_sub(OPEN_LINES) as i32;
                let d = if edit == Edit::PageUp {
                    OPEN_LINES as i32 - 1
                } else {
                    1 - OPEN_LINES as i32
                };
                chat.scroll = (chat.scroll as i32 + d).clamp(0, max) as usize;
            }
            Edit::Changed | Edit::Moved => {
                let (text, before) = (chat.field.text.clone(), chat.field.before().to_string());
                chat.suggest.refresh(&text, &before, &ctx);
            }
        }
        chat.dirty = true;
    }
    if !events.is_empty() {
        chat.dirty = true;
    }
}

// --- Drawing -----------------------------------------------------------------------------------

#[derive(Component)]
pub struct LinesBox;
#[derive(Component)]
struct InputBar;
#[derive(Component)]
struct InputText;
#[derive(Component)]
struct SuggestBox;
#[derive(Component)]
struct BarText;

fn spawn_ui(mut commands: Commands) {
    commands.spawn((
        LinesBox,
        Node {
            position_type: PositionType::Absolute,
            left: px(4),
            bottom: px(40),
            width: px(WIDTH),
            flex_direction: FlexDirection::Column,
            ..default()
        },
        GlobalZIndex(20),
    ));
    commands.spawn((
        SuggestBox,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(30),
            left: px(4),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Stretch,
            ..default()
        },
        GlobalZIndex(21),
        Visibility::Hidden,
    ));
    commands
        .spawn((
            InputBar,
            Node {
                position_type: PositionType::Absolute,
                left: px(2),
                right: px(2),
                bottom: px(2),
                height: px(24),
                padding: UiRect::horizontal(px(4)),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BACK),
            GlobalZIndex(20),
            Visibility::Hidden,
        ))
        .with_children(|b| {
            b.spawn((
                InputText,
                Text::default(),
                TextFont {
                    font_size: FontSize::Px(FONT),
                    ..default()
                },
                TextColor(WHITE),
            ));
        });
    commands.spawn((
        BarText,
        Text::default(),
        TextFont {
            font_size: FontSize::Px(17.0),
            ..default()
        },
        TextColor(WHITE),
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(110),
            width: percent(100),
            justify_content: JustifyContent::Center,
            ..default()
        },
        GlobalZIndex(19),
    ));
}

/// A piece of text with its own color, under a `Text`.
fn spans(parent: &mut ChildSpawnerCommands, pieces: &[(String, Color)], alpha: f32) {
    for (s, c) in pieces {
        parent.spawn((
            TextSpan::new(s.clone()),
            TextFont {
                font_size: FontSize::Px(FONT),
                ..default()
            },
            TextColor(c.with_alpha(alpha)),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_chat(
    mut commands: Commands,
    mut chat: ResMut<Chat>,
    lines_box: Single<Entity, With<LinesBox>>,
    mut bar: Single<&mut Visibility, (With<InputBar>, Without<SuggestBox>)>,
    input: Single<Entity, With<InputText>>,
    suggest_box: Single<
        (Entity, &mut Node, &mut Visibility),
        (With<SuggestBox>, Without<InputBar>),
    >,
    mut last_fade: Local<Vec<u8>>,
    mut last_blink: Local<bool>,
) {
    // Fading lines redraw as they fade (in steps); otherwise only when something changed.
    let now = chat.now;
    let shown: Vec<(usize, f32)> = if chat.open {
        let end = chat.lines.len().saturating_sub(chat.scroll);
        (end.saturating_sub(OPEN_LINES)..end)
            .map(|i| (i, 1.0))
            .collect()
    } else {
        let start = chat.lines.len().saturating_sub(CLOSED_LINES);
        (start..chat.lines.len())
            .filter_map(|i| {
                let age = now - chat.lines[i].at;
                let a = ((SHOW - age) / 1.0).clamp(0.0, 1.0);
                (a > 0.0).then_some((i, a))
            })
            .collect()
    };
    let fade: Vec<u8> = shown.iter().map(|&(_, a)| (a * 12.0) as u8).collect();
    let blink = chat.field.cursor_on();
    if !chat.dirty && fade == *last_fade && blink == *last_blink {
        return;
    }
    chat.dirty = false;
    *last_fade = fade;
    *last_blink = blink;

    commands.entity(*lines_box).despawn_related::<Children>();
    commands.entity(*lines_box).with_children(|col| {
        for &(i, a) in &shown {
            col.spawn((
                Node {
                    width: percent(100),
                    height: px(LINE_H),
                    padding: UiRect::left(px(3)),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(BACK.with_alpha(0.5 * a)),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::default(),
                    TextFont {
                        font_size: FontSize::Px(FONT),
                        ..default()
                    },
                ))
                .with_children(|t| spans(t, &chat.lines[i].spans, a));
            });
        }
    });

    **bar = if chat.open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    commands.entity(*input).despawn_related::<Children>();
    let (sbox, mut snode, mut svis) = suggest_box.into_inner();
    commands.entity(sbox).despawn_related::<Children>();
    if !chat.open {
        *svis = Visibility::Hidden;
        return;
    }
    // The line typed: red past where it stops making sense, the cursor, the rest of the
    // highlighted suggestion in gray.
    let f = &chat.field;
    let text: Vec<char> = f.text.chars().collect();
    let bad = chat.suggest.error_at().unwrap_or(usize::MAX);
    let mut pieces: Vec<(String, Color)> = Vec::new();
    let color_at = |i: usize| if i >= bad { RED } else { WHITE };
    let mut push = |s: String, c: Color| {
        if s.is_empty() {
            return;
        }
        match pieces.last_mut() {
            Some((t, lc)) if *lc == c => t.push_str(&s),
            _ => pieces.push((s, c)),
        }
    };
    for (i, ch) in text.iter().enumerate() {
        if i == f.cursor && blink {
            push("|".into(), WHITE);
        }
        push(ch.to_string(), color_at(i));
    }
    let ghost = chat.suggest.ghost(&f.text);
    if f.cursor == text.len() {
        if let Some(g) = ghost.filter(|g| !g.is_empty()) {
            push(g, DARK_GRAY);
            if blink {
                // Minecraft draws its cursor over the ghost text's start; a bar here keeps it readable.
                pieces.insert(pieces.len() - 1, ("|".into(), WHITE));
            }
        } else if blink {
            push("_".into(), WHITE);
        }
    }
    commands
        .entity(*input)
        .with_children(|t| spans(t, &pieces, 1.0));

    // Suggestions (or the usage of what comes next) above the line, under the word they complete.
    let rows = chat.suggest.rows();
    if rows.is_empty() {
        *svis = Visibility::Hidden;
        return;
    }
    *svis = Visibility::Inherited;
    snode.left = px(6.0 + chat.suggest.column() as f32 * CHAR_W);
    commands.entity(sbox).with_children(|col| {
        for (s, c) in rows {
            col.spawn((
                Node {
                    padding: UiRect::horizontal(px(3)),
                    height: px(LINE_H - 2.0),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(SUGGEST_BACK),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(s),
                    TextFont {
                        font_size: FontSize::Px(FONT),
                        ..default()
                    },
                    TextColor(c),
                ));
            });
        }
    });
}

fn draw_action_bar(
    mut bar: ResMut<ActionBar>,
    mut text: Single<(&mut Text, &mut TextColor), With<BarText>>,
    time: Res<Time>,
) {
    bar.left = (bar.left - time.delta_secs()).max(0.0);
    let a = bar.left.min(1.0);
    let want = if a > 0.0 {
        bar.text.clone()
    } else {
        String::new()
    };
    if text.0.0 != want {
        text.0.0 = want;
    }
    let c = WHITE.with_alpha(a);
    if text.1.0 != c {
        text.1.0 = c;
    }
}
