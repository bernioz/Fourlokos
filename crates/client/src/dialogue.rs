//! Talking to townsfolk: a right click on one opens a box at the bottom of the screen with their
//! name, what they say (typed out fast) and one to three replies, chosen with the keys 1 to 3 or
//! a click. Esc closes it; Enter or Space shows the rest of the line at once. While it is open it
//! owns the keyboard (`Owner::Dialogue`): nothing moves, nothing is mined, the mouse is free.
//! The tree and its rules are `orbit_core::dialogue`; the gifts are handed over here.

use crate::chat::{ActionBar, Keyboard, Owner};
use crate::inventory::{Drops, Frame};
use crate::player::Player;
use crate::story::Story;
use bevy::input::keyboard::Key;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::dialogue::{Dialogue, MAX_CHOICES, Npc};
use orbit_core::inventory::Hand;

/// Characters typed per second.
const TYPE_SPEED: f32 = 90.0;
const BOX_W: f32 = 780.0;
const YELLOW: Color = Color::srgb(1.0, 0.9, 0.35);

pub struct DialoguePlugin;

impl Plugin for DialoguePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Talk>().add_systems(
            Update,
            (
                open_talk
                    .after(crate::mobs::aim)
                    .before(crate::player::interact),
                run_talk.after(open_talk).before(crate::player::interact),
            ),
        );
    }
}

#[derive(Resource, Default)]
pub struct Talk {
    /// Set by the right click on a townsperson: who, and their mob id.
    pub request: Option<(Npc, u64)>,
    open: Option<Session>,
    /// What was said and chosen, for the log and the demos.
    pub log: Vec<String>,
}

impl Talk {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn node(&self) -> Option<usize> {
        self.open.as_ref().map(|s| s.dlg.node)
    }
}

struct Session {
    dlg: Dialogue,
    root: Entity,
    /// Characters of the line shown so far.
    shown: f32,
    regrab: bool,
}

#[derive(Component)]
struct LineText;
#[derive(Component)]
struct ChoiceButton(usize);
#[derive(Component)]
struct ChoiceText(usize);

fn open_talk(
    mut commands: Commands,
    mut talk: ResMut<Talk>,
    story: Res<Story>,
    mut keyboard: ResMut<Keyboard>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut player: Single<&mut Player>,
    mobs: Res<crate::mobs::Mobs>,
) {
    let Some((npc, mob)) = talk.request.take() else {
        return;
    };
    if talk.open.is_some() || keyboard.owner.is_some() {
        return;
    }
    // Turn to face them, as you would to talk.
    if let Some(m) = mobs.find(mob) {
        let b = &mut player.body;
        let d = m.mob.pos + m.mob.up() * 1.6 - b.eye();
        let flat = d - b.up * d.dot(b.up);
        b.forward = flat.normalize_or(b.forward);
        b.pitch = (d.dot(b.up) / d.length().max(1e-3)).clamp(-1.0, 1.0).asin();
    }
    let dlg = Dialogue::new(npc, &story.flags);
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                bottom: px(100),
                justify_content: JustifyContent::Center,
                ..default()
            },
            GlobalZIndex(40),
        ))
        .with_children(|r| {
            r.spawn((
                Node {
                    width: px(BOX_W),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(px(16)),
                    row_gap: px(8),
                    border: UiRect::all(px(2)),
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.06, 0.88)),
                BorderColor::all(Color::srgb(0.7, 0.62, 0.25)),
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(npc.name()),
                    TextFont {
                        font_size: FontSize::Px(22.0),
                        ..default()
                    },
                    TextColor(YELLOW),
                ));
                b.spawn((
                    LineText,
                    Text::new(""),
                    TextFont {
                        font_size: FontSize::Px(19.0),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    Node {
                        min_height: px(100),
                        ..default()
                    },
                ));
                for i in 0..MAX_CHOICES {
                    b.spawn((
                        ChoiceButton(i),
                        Node {
                            padding: UiRect::axes(px(8), px(3)),
                            border_radius: BorderRadius::all(px(3)),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        Visibility::Hidden,
                    ))
                    .with_child((
                        ChoiceText(i),
                        Text::new(""),
                        TextFont {
                            font_size: FontSize::Px(18.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.75, 0.9, 1.0)),
                    ));
                }
                b.spawn((
                    Text::new("[1-3] elegir   [Enter] mostrar   [Esc] cerrar"),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.5, 0.5, 0.55)),
                ));
            });
        })
        .id();
    talk.log.push(format!("opened {}", npc.name()));
    info!("dialogue: talking to {}", npc.name());
    talk.open = Some(Session {
        dlg,
        root,
        shown: 0.0,
        regrab: cursor.grab_mode != CursorGrabMode::None,
    });
    keyboard.owner = Some(Owner::Dialogue);
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
}

#[allow(clippy::too_many_arguments)]
fn run_talk(
    mut commands: Commands,
    mut talk: ResMut<Talk>,
    mut story: ResMut<Story>,
    mut keyboard: ResMut<Keyboard>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut player: Single<&mut Player>,
    mut drops: ResMut<Drops>,
    mut bar: ResMut<ActionBar>,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    buttons: Query<(&ChoiceButton, &ComputedNode, &UiGlobalTransform)>,
    mut line: Query<&mut Text, (With<LineText>, Without<ChoiceText>)>,
    mut texts: Query<(&ChoiceText, &mut Text), Without<LineText>>,
    mut vis: Query<(&ChoiceButton, &mut Visibility, &mut BackgroundColor)>,
) {
    let Some(s) = talk.open.as_mut() else { return };
    if keyboard.owner != Some(Owner::Dialogue) {
        return;
    }
    let text = s.dlg.node().text;
    let len = text.chars().count() as f32;
    s.shown = (s.shown + TYPE_SPEED * time.delta_secs()).min(len);
    let mut close = false;
    let mut pick: Option<usize> = None;
    for ev in std::mem::take(&mut keyboard.events) {
        if !ev.state.is_pressed() || ev.repeat {
            continue;
        }
        match (&ev.key_code, &ev.logical_key) {
            (KeyCode::Escape, _) | (_, Key::Escape) => close = true,
            (KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space, _)
            | (_, Key::Enter | Key::Space) => s.shown = len,
            (_, Key::Character(c)) => {
                if let Some(n) = c
                    .chars()
                    .next()
                    .and_then(|c| c.to_digit(10))
                    .filter(|n| (1..=MAX_CHOICES as u32).contains(n))
                {
                    // A key while the line is still being typed first shows all of it.
                    if s.shown < len {
                        s.shown = len;
                    } else {
                        pick = Some(n as usize - 1);
                    }
                }
            }
            _ => {}
        }
    }
    let done = s.shown >= len;
    // The button under the mouse (the cursor is in logical pixels, nodes in physical ones).
    let under = window.physical_cursor_position().and_then(|m| {
        buttons
            .iter()
            .find(|(b, n, g)| done && b.0 < s.dlg.choices().len() && n.contains_point(**g, m))
            .map(|(b, ..)| b.0)
    });
    if done && mouse.just_pressed(MouseButton::Left) {
        pick = under.or(pick);
    }
    if let Some(i) = pick {
        let chose = s.dlg.choices().get(i).map(|c| c.text);
        let npc = s.dlg.npc;
        if let Some(text) = chose {
            talk.log.push(format!("chose: {text}"));
            info!("dialogue: {} chose \"{text}\"", npc.name());
        }
        let s = talk.open.as_mut().unwrap();
        let delivery_hand = Hand::ORDER.into_iter().find(|&h| {
            player
                .inv
                .in_hand(h)
                .is_some_and(|stack| stack.item == orbit_core::items::COMPLETO_ITALIANO)
        });
        if let Some(result) = s
            .dlg
            .advance_with_item(i, &mut story.flags, delivery_hand.is_some())
        {
            if let Some(item) = result.delivered {
                if let Some(hand) = delivery_hand {
                    player.inv.use_hand(hand);
                    bar.show(&format!("Entregaste 1 x {}", orbit_core::items::name(item)));
                    talk.log
                        .push(format!("delivered: {}", orbit_core::items::name(item)));
                    story.dirty = true;
                }
            }
            if let Some(g) = result.gift {
                let stack = g.stack();
                let rest = player.inv.add(stack.item, stack.count);
                bar.show(&format!(
                    "Recibiste {} x {}",
                    stack.count,
                    orbit_core::items::name(stack.item)
                ));
                info!(
                    "dialogue: {} gave {} x {} (inventory left over: {rest})",
                    npc.name(),
                    stack.count,
                    orbit_core::items::name(stack.item)
                );
                talk.log.push(format!(
                    "gift: {} x {}",
                    stack.count,
                    orbit_core::items::name(stack.item)
                ));
                story.dirty = true;
                if rest > 0 {
                    let b = &player.body;
                    if let Some(id) = b.gravity_source {
                        for _ in 0..rest {
                            drops.pop(Frame::Planet(id), b.pos + b.up * 1.0, b.up, stack.item);
                        }
                    }
                }
            }
            let s = talk.open.as_mut().unwrap();
            s.shown = 0.0;
            if s.dlg.ended {
                close = true;
            }
        }
    }
    let Some(s) = talk.open.as_mut() else { return };
    // What is on screen: the typed part of the line, the replies once it is all there.
    let shown: String = s.dlg.node().text.chars().take(s.shown as usize).collect();
    for mut t in &mut line {
        if t.0 != shown {
            t.0 = shown.clone();
        }
    }
    let choices = s.dlg.choices();
    let done = s.shown >= s.dlg.node().text.chars().count() as f32;
    for (c, mut t) in &mut texts {
        let want = choices
            .get(c.0)
            .map_or(String::new(), |ch| format!("{}. {}", c.0 + 1, ch.text));
        if t.0 != want {
            t.0 = want;
        }
    }
    for (b, mut v, mut bg) in &mut vis {
        *v = if done && b.0 < choices.len() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        bg.0 = if under == Some(b.0) {
            Color::srgba(0.3, 0.4, 0.6, 0.6)
        } else {
            Color::NONE
        };
    }
    if close {
        let (root, regrab) = (s.root, s.regrab);
        commands.entity(root).despawn();
        talk.open = None;
        keyboard.owner = None;
        if regrab {
            cursor.grab_mode = CursorGrabMode::Locked;
            cursor.visible = false;
        }
        talk.log.push("closed".into());
        info!("dialogue: closed");
    }
}
