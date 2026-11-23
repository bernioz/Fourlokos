//! Commands typed in the chat, Minecraft (Brigadier) style: a tree of literal words and
//! arguments, suggestions while typing, `/help` with each command's usage, and errors that point
//! at where the line stops making sense (`...ion foo<--[HERE]`).
//!
//! - `/help [<command>]`
//! - `/station place [<name>]` puts a new station up in front of you (you must be in orbit).
//! - `/station list`, `/station info`, `/station rename <name>`, `/station pack`,
//!   `/station restore <id>` (a packed station back in front of you), `/station tp <id>`,
//!   `/station height [sync|<blocks>]`: its orbit's height and lanes, or a move to another lane
//!   (higher goes round slower; `sync` keeps over one spot of the ground).
//!   `/fourlokos station ...` is the same, as `/galaxycraft station ...` is in GalaxyCraft.
//! - `/elevator build`: a hologram of a space elevator where you look on the ground, on the
//!   equator; right click builds it (its platform in blocks, its station up in the synchronous
//!   lane over it), left click (or the command again) puts it away.
//!   `/elevator info`, `/elevator list`, `/elevator remove`.
//! - `/schematic` opens the schematics menu (each one turning in 3D, with Place).
//! - `/schematic save [<name>]` saves the station you are on to `schematics/<name>.obsc`;
//!   `/schematic list`; `/schematic place <file> [<name>]` puts a new station up from one.
//! - `/rocket build [<name>]` puts the starter rocket on a flat 3x3 pad in front of you;
//!   `/rocket leave` lets go of the helm (as R does). See `rocket`.
//! - `/tp <station or planet>`
//! - `/time set (day|noon|night|midnight|<seconds>)`, `/time add <seconds>`,
//!   `/time query (daytime|gametime|day)`: day and night are the planet's you are at.
//! - `/eclipse`: when the next eclipses are, where you are.
//! - `/gamemode (survival|creative)` (or `s`, `c`, `0`, `1`): survival places blocks out of the
//!   inventory and breaking them drops items; creative has every item, without end, and flies.
//! - `/give <item> [<count>]`: items into the inventory (names as `/give` suggests: `oak_door`).
//! - `/clear`: empties the inventory.
//! - `/seed`, `/say <message>`, `/me <action>`

use crate::elevator::{self, Elevators};
use crate::chat::{ActionBar, Chat, AQUA, GRAY, GREEN, RED, WHITE, YELLOW};
use crate::player::Player;
use crate::sky::SkySeed;
use crate::station::Stations;
use crate::travel::Travel;
use crate::world::{Clock, Space, TimeShift};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use orbit_core::planet::PlanetSpec;
use orbit_core::player::Body;
use orbit_core::schematic::{self, Schematic};
use orbit_core::station::{self, Lanes, Refusal, StationOrbit};
use std::f32::consts::PI;

/// Where schematics are kept, relative to where the game runs.
const SCHEMATICS: &str = "schematics";

/// Puts a new station up in front of the player, built from schematic `file`: what to say in
/// the chat, or why it could not.
pub fn place_schematic(stations: &mut Stations, body: &Body, space: &Space, t: f64, file: &str, name: Option<&str>) -> Result<String, String> {
    let (_, sc) = schematic_files().into_iter().find(|(stem, _)| stem == file).ok_or_else(|| format!("No schematic {file}"))?;
    let i = stations.add(&sc, name, t);
    match stations.deploy(Some(i), None, body, space, t) {
        Ok(_) => {
            let s = stations.list[i].station.read().unwrap();
            let o = s.orbit.unwrap();
            let planet = space.planets.get(&o.planet).map_or("a far planet".into(), |p| p.spec.name.clone());
            Ok(format!("Placed {} ({}) from {file} in orbit around {planet}", s.name, s.id))
        }
        Err(r) => {
            stations.drop_last();
            Err(r.message().into())
        }
    }
}

/// The schematics saved (file stem, contents), by name; files that do not read are left out.
pub fn schematic_files() -> Vec<(String, Schematic)> {
    let Ok(dir) = std::fs::read_dir(SCHEMATICS) else { return vec![] };
    let mut v: Vec<(String, Schematic)> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == schematic::EXT))
        .filter_map(|p| Some((p.file_stem()?.to_str()?.to_string(), Schematic::from_bytes(&std::fs::read(&p).ok()?).ok()?)))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

/// A time span in words ("4 min 05 s").
fn duration(s: f64) -> String {
    crate::map::fmt_time(s as f32)
}

/// An item's name for `/give`, as Minecraft writes them: `oak_door`, `cobblestone`.
pub fn item_id(b: orbit_core::block::Block) -> String {
    orbit_core::block::name(b).to_lowercase().replace(' ', "_")
}

/// The player's name in the chat.
pub const PLAYER: &str = "Player";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cmd {
    Help,
    StationPlace,
    StationList,
    StationInfo,
    StationRename,
    StationPack,
    StationRestore,
    StationTp,
    StationHeight,
    StationHeightSync,
    ElevatorBuild,
    ElevatorInfo,
    ElevatorList,
    ElevatorRemove,
    SchematicMenu,
    SchematicSave,
    SchematicList,
    SchematicPlace,
    RocketBuild,
    RocketLeave,
    Tp,
    TimeSet,
    TimeAdd,
    TimeQuery,
    GameMode,
    Give,
    Clear,
    Seed,
    Eclipse,
    Say,
    Me,
}

/// What an argument suggests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hint {
    None,
    Commands,
    Placed,
    Packed,
    Destinations,
    Schematics,
    Items,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Literal(&'static str),
    /// One word.
    Word(&'static str, Hint),
    /// The rest of the line.
    Greedy(&'static str, Hint),
    Number(&'static str),
}

struct Node {
    kind: Kind,
    run: Option<Cmd>,
    kids: Vec<Node>,
}

fn lit(name: &'static str) -> Node {
    Node { kind: Kind::Literal(name), run: None, kids: vec![] }
}

fn word(name: &'static str, hint: Hint) -> Node {
    Node { kind: Kind::Word(name, hint), run: None, kids: vec![] }
}

fn greedy(name: &'static str, hint: Hint) -> Node {
    Node { kind: Kind::Greedy(name, hint), run: None, kids: vec![] }
}

fn number(name: &'static str) -> Node {
    Node { kind: Kind::Number(name), run: None, kids: vec![] }
}

impl Node {
    fn runs(mut self, c: Cmd) -> Self {
        self.run = Some(c);
        self
    }

    fn then(mut self, n: Node) -> Self {
        self.kids.push(n);
        self
    }

    fn usage(&self) -> String {
        match self.kind {
            Kind::Literal(l) => l.into(),
            Kind::Word(n, _) | Kind::Greedy(n, _) | Kind::Number(n) => format!("<{n}>"),
        }
    }

    fn literal(&self) -> Option<&'static str> {
        match self.kind {
            Kind::Literal(l) => Some(l),
            _ => None,
        }
    }
}

fn station_tree(root: &'static str) -> Node {
    lit(root)
        .then(lit("place").runs(Cmd::StationPlace).then(greedy("name", Hint::None).runs(Cmd::StationPlace)))
        .then(lit("list").runs(Cmd::StationList))
        .then(lit("info").runs(Cmd::StationInfo))
        .then(lit("rename").then(greedy("name", Hint::None).runs(Cmd::StationRename)))
        .then(lit("pack").runs(Cmd::StationPack))
        .then(lit("restore").then(word("id", Hint::Packed).runs(Cmd::StationRestore)))
        .then(lit("tp").then(word("id", Hint::Placed).runs(Cmd::StationTp)))
        .then(lit("height").runs(Cmd::StationHeight).then(lit("sync").runs(Cmd::StationHeightSync)).then(number("blocks").runs(Cmd::StationHeight)))
}

/// Every command.
fn tree() -> Vec<Node> {
    vec![
        lit("help").runs(Cmd::Help).then(word("command", Hint::Commands).runs(Cmd::Help)),
        station_tree("station"),
        lit("fourlokos").then(station_tree("station")),
        lit("elevator")
            .then(lit("build").runs(Cmd::ElevatorBuild))
            .then(lit("info").runs(Cmd::ElevatorInfo))
            .then(lit("list").runs(Cmd::ElevatorList))
            .then(lit("remove").runs(Cmd::ElevatorRemove)),
        lit("schematic")
            .runs(Cmd::SchematicMenu)
            .then(lit("save").runs(Cmd::SchematicSave).then(greedy("name", Hint::None).runs(Cmd::SchematicSave)))
            .then(lit("list").runs(Cmd::SchematicList))
            .then(lit("place").then(word("file", Hint::Schematics).runs(Cmd::SchematicPlace).then(greedy("name", Hint::None).runs(Cmd::SchematicPlace)))),
        lit("rocket").then(lit("build").runs(Cmd::RocketBuild).then(greedy("name", Hint::None).runs(Cmd::RocketBuild))).then(lit("leave").runs(Cmd::RocketLeave)),
        lit("tp").then(greedy("destination", Hint::Destinations).runs(Cmd::Tp)),
        lit("time")
            .then(
                lit("set")
                    .then(lit("day").runs(Cmd::TimeSet))
                    .then(lit("noon").runs(Cmd::TimeSet))
                    .then(lit("night").runs(Cmd::TimeSet))
                    .then(lit("midnight").runs(Cmd::TimeSet))
                    .then(number("time").runs(Cmd::TimeSet)),
            )
            .then(lit("add").then(number("time").runs(Cmd::TimeAdd)))
            .then(lit("query").then(lit("daytime").runs(Cmd::TimeQuery)).then(lit("gametime").runs(Cmd::TimeQuery)).then(lit("day").runs(Cmd::TimeQuery))),
        lit("gamemode")
            .then(lit("survival").runs(Cmd::GameMode))
            .then(lit("creative").runs(Cmd::GameMode))
            .then(word("mode", Hint::None).runs(Cmd::GameMode)),
        lit("give").then(word("item", Hint::Items).runs(Cmd::Give).then(number("count").runs(Cmd::Give))),
        lit("clear").runs(Cmd::Clear),
        lit("seed").runs(Cmd::Seed),
        lit("eclipse").runs(Cmd::Eclipse),
        lit("say").then(greedy("message", Hint::None).runs(Cmd::Say)),
        lit("me").then(greedy("action", Hint::None).runs(Cmd::Me)),
    ]
}

struct Parsed {
    cmd: Cmd,
    /// The words of the line, after the slash.
    words: Vec<String>,
    /// Argument values, in order.
    args: Vec<String>,
}

/// Why a line is not a command, and where (in characters, after the slash).
#[derive(Debug, PartialEq)]
struct ParseError {
    message: String,
    at: usize,
    /// Only missing something at the end (not wrong).
    incomplete: bool,
}

const UNKNOWN: &str = "Unknown or incomplete command, see below for error";
const INCORRECT: &str = "Incorrect argument for command";

/// Parses a line (without its slash).
fn parse(roots: &[Node], s: &str) -> Result<Parsed, ParseError> {
    let chars: Vec<char> = s.chars().collect();
    let mut args = Vec::new();
    let mut words = Vec::new();
    let mut kids = roots;
    let mut runs: Option<Cmd> = None;
    let mut pos = 0;
    let mut root = true;
    loop {
        if pos >= chars.len() {
            return match runs {
                Some(cmd) if !root => Ok(Parsed { cmd, words, args }),
                _ => Err(ParseError { message: UNKNOWN.into(), at: chars.len(), incomplete: true }),
            };
        }
        let end = chars[pos..].iter().position(|&c| c == ' ').map_or(chars.len(), |i| pos + i);
        let token: String = chars[pos..end].iter().collect();
        let next = if end < chars.len() { end + 1 } else { end };
        let fail = |at: usize| ParseError { message: if root { UNKNOWN } else { INCORRECT }.into(), at, incomplete: false };
        let node = kids.iter().find(|k| k.literal() == Some(token.as_str())).or_else(|| kids.iter().find(|k| k.literal().is_none()));
        let Some(node) = node else { return Err(fail(pos)) };
        match node.kind {
            Kind::Literal(_) => {}
            Kind::Word(..) if token.is_empty() => return Err(fail(pos)),
            Kind::Word(..) => args.push(token.clone()),
            Kind::Greedy(..) => {
                let rest: String = chars[pos..].iter().collect();
                if rest.trim().is_empty() {
                    return Err(fail(pos));
                }
                args.push(rest.clone());
                words.push(rest);
                return node.run.map(|cmd| Parsed { cmd, words, args }).ok_or(ParseError { message: UNKNOWN.into(), at: chars.len(), incomplete: true });
            }
            Kind::Number(_) => match token.parse::<f64>() {
                Ok(v) if v.is_finite() => args.push(token.clone()),
                _ if token.is_empty() => return Err(ParseError { message: "Expected double".into(), at: pos, incomplete: false }),
                _ => return Err(ParseError { message: format!("Invalid double '{token}'"), at: pos, incomplete: false }),
            },
        }
        words.push(token);
        runs = node.run;
        kids = &node.kids;
        root = false;
        // A space at the very end wants something after it.
        if next == chars.len() && end < chars.len() {
            return Err(ParseError { message: UNKNOWN.into(), at: chars.len(), incomplete: true });
        }
        pos = next;
        if pos < chars.len() && kids.is_empty() {
            return Err(ParseError { message: INCORRECT.into(), at: pos, incomplete: false });
        }
    }
}

/// The usage of what may follow a node, Brigadier's "smart usage": `(a|b)`, `<x>`, `[...]` when
/// it may stop there.
fn smart(node: &Node) -> String {
    let inner = match node.kids.len() {
        0 => return String::new(),
        1 => {
            let k = &node.kids[0];
            let more = smart(k);
            if more.is_empty() { k.usage() } else { format!("{} {}", k.usage(), more) }
        }
        _ => {
            let all: Vec<String> = node.kids.iter().map(Node::usage).collect();
            if node.run.is_some() { all.join("|") } else { format!("({})", all.join("|")) }
        }
    };
    if node.run.is_some() { format!("[{inner}]") } else { inner }
}

/// Every way a command can be run, one per line.
fn all_usage(node: &Node, prefix: &str, out: &mut Vec<String>) {
    let here = if prefix.is_empty() { node.usage() } else { format!("{prefix} {}", node.usage()) };
    if node.run.is_some() {
        out.push(format!("/{here}"));
    }
    for k in &node.kids {
        all_usage(k, &here, out);
    }
}

// --- Suggestions while typing ------------------------------------------------------------------

/// What suggestions can name.
#[derive(SystemParam)]
pub struct Context<'w> {
    stations: Res<'w, Stations>,
    space: Res<'w, Space>,
}

impl Context<'_> {
    fn values(&self, hint: Hint, roots: &[Node]) -> Vec<String> {
        match hint {
            Hint::None => vec![],
            Hint::Commands => roots.iter().filter_map(|n| n.literal().map(String::from)).collect(),
            Hint::Placed => self.stations.ids(Some(true)),
            Hint::Packed => self.stations.ids(Some(false)),
            Hint::Schematics => schematic_files().into_iter().map(|(stem, _)| stem).collect(),
            Hint::Items => orbit_core::block::ITEMS.iter().map(|&b| item_id(b)).collect(),
            Hint::Destinations => {
                let mut v = self.stations.ids(Some(true));
                v.extend(self.space.planets.values().map(|p| p.spec.name.clone()));
                v
            }
        }
    }
}

#[derive(Default)]
pub struct Suggestions {
    list: Vec<String>,
    selected: usize,
    /// Where the word being completed starts (characters, slash included).
    start: usize,
    shown: bool,
    /// When nothing to suggest: what comes next (`<name>`).
    usage: Vec<String>,
    /// Where the line stops making sense, and why.
    error: Option<(usize, String)>,
    /// Tab put a suggestion in; Tab again puts in the next.
    tabbed: bool,
}

impl Suggestions {
    pub fn visible(&self) -> bool {
        self.shown && !self.list.is_empty()
    }

    pub fn hide(&mut self) {
        self.shown = false;
    }

    pub fn select(&mut self, d: i32) {
        if !self.list.is_empty() {
            self.selected = (self.selected as i32 + d).rem_euclid(self.list.len() as i32) as usize;
            self.tabbed = false;
        }
    }

    /// Recomputes them for the line `text`, typed up to `before`.
    pub fn refresh(&mut self, text: &str, before: &str, ctx: &Context) {
        *self = Suggestions::default();
        let Some(line) = text.strip_prefix('/') else { return };
        let roots = tree();
        if let Err(e) = parse(&roots, line) {
            if !e.incomplete {
                self.error = Some((e.at + 1, e.message));
            }
        }
        let Some(typed) = before.strip_prefix('/') else { return };
        let chars: Vec<char> = typed.chars().collect();
        let mut kids: &[Node] = &roots;
        let mut pos = 0;
        // Walk the words already typed out, to what the last (partial) word may be.
        while let Some(i) = chars[pos..].iter().position(|&c| c == ' ') {
            let token: String = chars[pos..pos + i].iter().collect();
            let node = kids.iter().find(|k| k.literal() == Some(token.as_str())).or_else(|| kids.iter().find(|k| k.literal().is_none()));
            let Some(node) = node else { return };
            if let Kind::Greedy(_, hint) = node.kind {
                // The rest of the line is this argument, spaces and all.
                let partial: String = chars[pos..].iter().collect();
                self.offer(pos, &partial, ctx.values(hint, &roots), vec![]);
                return;
            }
            kids = &node.kids;
            pos += i + 1;
        }
        let partial: String = chars[pos..].iter().collect();
        let mut options = Vec::new();
        let mut usage = Vec::new();
        for k in kids {
            match k.kind {
                Kind::Literal(l) => options.push(l.to_string()),
                Kind::Word(_, hint) | Kind::Greedy(_, hint) => {
                    let v = ctx.values(hint, &roots);
                    if v.is_empty() {
                        usage.push(k.usage());
                    }
                    options.extend(v);
                }
                Kind::Number(_) => usage.push(k.usage()),
            }
        }
        self.offer(pos, &partial, options, usage);
    }

    fn offer(&mut self, pos: usize, partial: &str, mut options: Vec<String>, usage: Vec<String>) {
        let p = partial.to_lowercase();
        options.retain(|o| o.to_lowercase().starts_with(&p) && o != partial);
        options.sort();
        options.dedup();
        self.start = pos + 1;
        self.list = options;
        self.shown = true;
        if partial.is_empty() {
            self.usage = usage;
        }
    }

    /// The line with the highlighted suggestion put in (Tab), or None if there is none.
    pub fn complete(&mut self, text: &str) -> Option<String> {
        if self.list.is_empty() {
            return None;
        }
        if self.tabbed {
            self.selected = (self.selected + 1) % self.list.len();
        }
        self.shown = true;
        let head: String = text.chars().take(self.start).collect();
        Some(format!("{head}{}", self.list[self.selected]))
    }

    pub fn keep_after_tab(&mut self) {
        self.tabbed = true;
        self.error = None;
    }

    /// The rest of the highlighted suggestion after what is typed, in gray after the cursor.
    pub fn ghost(&self, text: &str) -> Option<String> {
        if !self.visible() || self.tabbed {
            return None;
        }
        let typed: String = text.chars().skip(self.start).collect();
        let s = &self.list[self.selected];
        s.to_lowercase().starts_with(&typed.to_lowercase()).then(|| s.chars().skip(typed.chars().count()).collect())
    }

    /// Lines to show above the input: suggestions (the highlighted one yellow), else what comes
    /// next, else the error.
    pub fn rows(&self) -> Vec<(String, Color)> {
        if self.visible() {
            const MAX: usize = 10;
            let n = self.list.len();
            let first = self.selected.saturating_sub(MAX - 1).min(n.saturating_sub(MAX));
            let mut rows: Vec<(String, Color)> = (first..n.min(first + MAX)).map(|i| (self.list[i].clone(), if i == self.selected { YELLOW } else { GRAY })).collect();
            if first > 0 {
                rows.insert(0, ("...".into(), GRAY));
            }
            if first + MAX < n {
                rows.push(("...".into(), GRAY));
            }
            return rows;
        }
        if let Some((_, e)) = &self.error {
            return vec![(e.clone(), RED)];
        }
        if self.shown && !self.usage.is_empty() {
            return vec![(self.usage.join(" | "), GRAY)];
        }
        vec![]
    }

    /// The column the rows line up under.
    pub fn column(&self) -> usize {
        if self.visible() || self.error.is_none() { self.start } else { 0 }
    }

    pub fn error_at(&self) -> Option<usize> {
        self.error.as_ref().map(|e| e.0)
    }
}

// --- Running them ------------------------------------------------------------------------------

/// The feedback for a line that is not a command: the message, then the line up to where it
/// went wrong (gray), the rest (red) and `<--[HERE]`.
fn report(chat: &mut Chat, line: &str, e: &ParseError) {
    chat.error(&e.message);
    let chars: Vec<char> = line.chars().collect();
    let at = e.at.min(chars.len());
    let mut pieces = Vec::new();
    let from = at.saturating_sub(10);
    let head: String = chars[from..at].iter().collect();
    pieces.push((if from > 0 { format!("...{head}") } else { head }, GRAY));
    if at < chars.len() {
        pieces.push((chars[at..].iter().collect(), RED));
    }
    pieces.push(("<--[HERE]".into(), RED));
    chat.line(pieces);
}

/// Runs the lines sent in the chat: commands, or messages from the player.
#[allow(clippy::too_many_arguments)]
pub fn run(
    mut chat: ResMut<Chat>,
    mut stations: ResMut<Stations>,
    space: Res<Space>,
    mut player: Single<&mut Player>,
    clock: Res<Clock>,
    mut shift: ResMut<TimeShift>,
    mut travel: ResMut<Travel>,
    seed: Res<SkySeed>,
    mut bar: ResMut<ActionBar>,
    mut schematic_menu: ResMut<crate::schematic_menu::SchematicMenu>,
    mut elevators: ResMut<Elevators>,
    mut rockets: ResMut<crate::rocket::Rockets>,
) {
    let roots = tree();
    for line in std::mem::take(&mut chat.pending) {
        let Some(cmd) = line.strip_prefix('/') else {
            chat.info(&format!("<{PLAYER}> {line}"));
            continue;
        };
        let p = match parse(&roots, cmd) {
            Ok(p) => p,
            Err(e) => {
                report(&mut chat, cmd, &e);
                continue;
            }
        };
        let mut ctx = Run { chat: &mut chat, stations: &mut stations, space: &space, player: &mut player, t: clock.0, shift: &mut shift, travel: &mut travel, bar: &mut bar, elevators: &mut elevators };
        match p.cmd {
            Cmd::Help => help(ctx.chat, &roots, p.args.first().map(String::as_str)),
            Cmd::StationPlace => ctx.place(p.args.first().map(String::as_str)),
            Cmd::StationList => ctx.list(),
            Cmd::StationInfo => ctx.info(),
            Cmd::StationRename => ctx.rename(&p.args[0]),
            Cmd::StationPack => ctx.pack(),
            Cmd::StationRestore => ctx.restore(&p.args[0]),
            Cmd::StationTp => ctx.tp_station(&p.args[0]),
            Cmd::SchematicMenu => schematic_menu.wanted = true,
            Cmd::SchematicSave => ctx.schematic_save(p.args.first().map(String::as_str)),
            Cmd::SchematicList => ctx.schematic_list(),
            Cmd::SchematicPlace => ctx.schematic_place(&p.args[0], p.args.get(1).map(String::as_str)),
            Cmd::StationHeight => ctx.height(p.args.first().map(|a| a.parse().unwrap()), false),
            Cmd::StationHeightSync => ctx.height(None, true),
            Cmd::ElevatorBuild => ctx.elevator_build(),
            Cmd::ElevatorInfo => ctx.elevator_info(),
            Cmd::ElevatorList => ctx.elevator_list(),
            Cmd::ElevatorRemove => ctx.elevator_remove(),
            Cmd::RocketBuild => rockets.requests.push(crate::rocket::Request::Build(p.args.first().cloned())),
            Cmd::RocketLeave => rockets.requests.push(crate::rocket::Request::Release),
            Cmd::Tp => ctx.tp(p.args[0].trim()),
            Cmd::TimeSet => ctx.time_set(p.words.last().unwrap()),
            Cmd::TimeAdd => {
                let v: f64 = p.args[0].parse().unwrap();
                ctx.shift.0 += v;
                let now = ctx.t + v;
                ctx.chat.info(&format!("Set the time to {}", now.round() as i64));
            }
            Cmd::TimeQuery => ctx.time_query(p.words.last().unwrap()),
            Cmd::Eclipse => ctx.eclipse(),
            Cmd::GameMode => match orbit_core::inventory::GameMode::parse(p.words.last().unwrap()) {
                Some(m) => {
                    ctx.player.inv.mode = m;
                    if m == orbit_core::inventory::GameMode::Survival {
                        ctx.player.body.flying = false;
                    }
                    ctx.chat.info(&format!("Set own game mode to {} Mode", m.name()));
                }
                None => ctx.chat.error(&format!("Unknown game mode: {}", p.words.last().unwrap())),
            },
            Cmd::Give => {
                let name = p.args[0].trim().to_lowercase();
                let count: u32 = p.args.get(1).and_then(|c| c.parse::<f64>().ok()).map_or(1, |c| c.clamp(1.0, 64.0 * 36.0) as u32);
                match orbit_core::block::ITEMS.iter().find(|&&b| item_id(b) == name) {
                    Some(&b) => {
                        let mut left = count;
                        while left > 0 {
                            let n = left.min(64) as u8;
                            let rest = ctx.player.inv.add(b, n);
                            left -= (n - rest) as u32;
                            if rest > 0 {
                                break;
                            }
                        }
                        ctx.chat.info(&format!("Gave {} [{}] to {PLAYER}", count - left, orbit_core::block::name(b)));
                    }
                    None => ctx.chat.error(&format!("Unknown item '{name}'")),
                }
            }
            Cmd::Clear => {
                let n: u32 = ctx.player.inv.slots.iter().flatten().map(|s| s.count as u32).sum();
                ctx.player.inv.slots = [None; orbit_core::inventory::SLOTS];
                ctx.chat.info(&format!("Removed {n} item(s) from player {PLAYER}"));
            }
            Cmd::Seed => ctx.chat.line(vec![("Seed: [".into(), WHITE), (seed.0.to_string(), GREEN), ("]".into(), WHITE)]),
            Cmd::Say => ctx.chat.info(&format!("[{PLAYER}] {}", p.args[0])),
            Cmd::Me => ctx.chat.info(&format!("* {PLAYER} {}", p.args[0])),
        }
    }
}

fn help(chat: &mut Chat, roots: &[Node], on: Option<&str>) {
    match on {
        None => {
            let mut lines: Vec<String> = roots.iter().map(|n| format!("/{} {}", n.usage(), smart(n)).trim_end().to_string()).collect();
            lines.sort();
            for l in lines {
                chat.info(&l);
            }
        }
        Some(name) => match roots.iter().find(|n| n.literal() == Some(name)) {
            Some(n) => {
                let mut out = Vec::new();
                all_usage(n, "", &mut out);
                for l in out {
                    chat.info(&l);
                }
            }
            None => chat.error("Unknown command or insufficient permissions"),
        },
    }
}

struct Run<'a> {
    chat: &'a mut Chat,
    stations: &'a mut Stations,
    space: &'a Space,
    player: &'a mut Player,
    t: f64,
    shift: &'a mut TimeShift,
    travel: &'a mut Travel,
    bar: &'a mut ActionBar,
    elevators: &'a mut Elevators,
}

impl Run<'_> {
    fn planet_name(&self, id: u64) -> String {
        self.space.planets.get(&id).map_or("a far planet".into(), |p| p.spec.name.clone())
    }

    /// Where a station is, for lists: "packed", or around which planet.
    fn whereabouts(&self, i: usize) -> String {
        let s = self.stations.list[i].station.read().unwrap();
        match s.orbit {
            None => "packed".into(),
            Some(o) => format!("around {}", self.planet_name(o.planet)),
        }
    }

    /// The station you are on, else the nearest one within reach of a command.
    fn here(&mut self) -> Option<usize> {
        let found = self.stations.near(&self.player.body, self.space, self.t, 300.0);
        if found.is_none() {
            self.chat.error("You are not on or near a station");
        }
        found
    }

    fn place(&mut self, name: Option<&str>) {
        match self.stations.deploy(None, name, &self.player.body, self.space, self.t) {
            Ok(i) => {
                let s = self.stations.list[i].station.read().unwrap();
                let planet = self.planet_name(s.orbit.unwrap().planet);
                self.chat.info(&format!("Placed {} ({}) in orbit around {}", s.name, s.id, planet));
            }
            Err(r) => self.chat.error(r.message()),
        }
    }

    fn list(&mut self) {
        if self.stations.list.is_empty() {
            self.chat.info("No stations yet");
            return;
        }
        for i in 0..self.stations.list.len() {
            let w = self.whereabouts(i);
            let s = self.stations.list[i].station.read().unwrap();
            let span = s.bounds.span();
            let line = format!("{} \"{}\": {}, {} blocks, {} x {} x {}", s.id, s.name, w, s.block_count(), span.x, span.z, span.y);
            drop(s);
            self.chat.info(&line);
        }
    }

    /// The lanes around a planet, if it is streamed in.
    fn lanes(&self, planet: u64) -> Option<(Lanes, f32)> {
        let p = self.space.planets.get(&planet)?;
        Some((crate::station::lanes_of(self.space, planet, &self.stations.tethers)?.0, p.spec.radius))
    }

    /// Where a station's orbit is, in words: its height, how long a turn takes, and its lane.
    fn orbit_line(&self, o: &StationOrbit) -> String {
        let planet = self.planet_name(o.planet);
        let Some((lanes, ground)) = self.lanes(o.planet) else {
            return format!("In orbit around {planet}, once every {}", duration(o.period()));
        };
        let k = lanes.radii.iter().position(|r| (r - o.radius()).abs() < 0.5);
        let lane = match k {
            Some(_) if lanes.is_sync(o.radius()) => ", the synchronous lane".to_string(),
            Some(k) => format!(", lane {} of {}", k + 1, lanes.radii.len()),
            None => String::new(),
        };
        format!("Around {planet}, {:.0} blocks up{lane}: once every {}", o.radius() - ground, duration(o.period()))
    }

    fn info(&mut self) {
        let Some(i) = self.here() else { return };
        let s = self.stations.list[i].station.read().unwrap();
        let o = s.orbit.unwrap();
        let span = s.bounds.span();
        let (name, id, blocks) = (s.name.clone(), s.id.clone(), s.block_count());
        drop(s);
        self.chat.line(vec![(format!("{name} "), YELLOW), (format!("({id})"), GRAY)]);
        let line = self.orbit_line(&o);
        self.chat.info(&line);
        self.chat.info(&format!("{} x {} x {}, {blocks} blocks", span.x, span.z, span.y));
    }

    /// Shows the station's height and the lanes it could go to, or moves it to another lane:
    /// the one nearest `altitude` (blocks over the surface), or with `sync` the synchronous one.
    fn height(&mut self, altitude: Option<f32>, sync: bool) {
        let Some(i) = self.here() else { return };
        let Some(o) = self.stations.list[i].station.read().unwrap().orbit else { return };
        if altitude.is_none() && !sync {
            let line = self.orbit_line(&o);
            self.chat.info(&line);
            if let Some((lanes, ground)) = self.lanes(o.planet) {
                let (lo, hi) = (lanes.radii[0] - ground, lanes.radii[lanes.radii.len() - 1] - ground);
                self.chat.line(vec![(format!("Lanes from {lo:.0} to {hi:.0} blocks up, every {:.0}: ", station::LANE), WHITE), ("/station height <blocks>".into(), AQUA)]);
                match lanes.sync {
                    Some(r) => self.chat.line(vec![(format!("Synchronous (one turn a day, over one spot): {:.0} blocks up, ", r - ground), WHITE), ("/station height sync".into(), AQUA)]),
                    None => self.chat.line(vec![(Refusal::NoSync.message().into(), GRAY)]),
                }
            }
            return;
        }
        let name = self.stations.list[i].station.read().unwrap().name.clone();
        if self.held_by_elevator(i) {
            self.chat.line(vec![(format!("{name} holds a space elevator up: it stays over its anchor. Take it down first with "), RED), ("/elevator remove".into(), AQUA)]);
            return;
        }
        match self.stations.move_to(i, if sync { None } else { altitude }, self.space, self.t) {
            Ok(o) => {
                self.stations.mark_dirty(i);
                let line = self.orbit_line(&o);
                self.chat.info(&format!("Moved {name}. {line}"));
            }
            Err(r) => self.chat.error(r.message()),
        }
    }

    /// Saves the station you are on (or near) as a schematic file.
    fn schematic_save(&mut self, name: Option<&str>) {
        let Some(i) = self.here() else { return };
        let sc = self.stations.list[i].station.read().unwrap().to_schematic();
        let stem = schematic::file_stem(name.unwrap_or(&sc.name));
        let path = std::path::Path::new(SCHEMATICS).join(format!("{stem}.{}", schematic::EXT));
        let saved = std::fs::create_dir_all(SCHEMATICS).and_then(|_| std::fs::write(&path, sc.to_bytes()));
        match saved {
            Ok(()) => {
                let n = sc.solid_cells().count();
                self.chat.line(vec![(format!("Saved {} ({n} blocks) as ", sc.name), WHITE), (stem.clone(), AQUA), (format!(": /schematic place {stem}"), GRAY)]);
            }
            Err(e) => self.chat.error(&format!("Could not save {}: {e}", path.display())),
        }
    }

    fn schematic_list(&mut self) {
        let files = schematic_files();
        if files.is_empty() {
            self.chat.info("No schematics yet: /schematic save on a station");
        }
        for (stem, sc) in files {
            let span = sc.size;
            self.chat.line(vec![(stem, AQUA), (format!(" \"{}\": {} blocks, {} x {} x {}", sc.name, sc.solid_cells().count(), span.x, span.z, span.y), WHITE)]);
        }
    }

    /// Puts a new station up in front of you, built from a schematic.
    fn schematic_place(&mut self, file: &str, name: Option<&str>) {
        match place_schematic(self.stations, &self.player.body, self.space, self.t, file, name) {
            Ok(line) => self.chat.info(&line),
            Err(e) => self.chat.error(&e),
        }
    }

    /// When the next eclipses are, at the planet or moon you are at.
    fn eclipse(&mut self) {
        let Some(id) = self.player.body.planet() else {
            self.chat.error("You are not at a planet");
            return;
        };
        let Some(here) = self.space.planets.get(&id) else { return };
        // The planet and its moons: this one's, or this moon's planet's.
        let host_id = here.spec.host.map_or(id, |h| h.id);
        let Some(host) = self.space.planets.get(&host_id) else { return };
        let moons: Vec<_> = self.space.planets.values().filter(|m| m.spec.host.is_some_and(|h| h.id == host_id) && (here.spec.host.is_none() || m.spec.id == id)).collect();
        if moons.is_empty() {
            self.chat.info(&format!("{} has no moons: no eclipses here", host.spec.name));
            return;
        }
        for m in moons {
            let (p, mo) = (host.spec.clone(), m.spec.clone());
            let at = |t: f64| {
                let (mut p, mut mo) = (p.clone(), mo.clone());
                p.advance(t);
                mo.advance(t);
                (p.star(), p.center, mo.center)
            };
            let radii = (host.star_radius as f64, p.radius as f64, mo.radius as f64);
            match orbit_core::eclipse::next_eclipse(at, radii, self.t, 6.0 * 3600.0) {
                Some((when, kind, deep)) => {
                    let (what, place) = match kind {
                        orbit_core::eclipse::Kind::Solar => ("Solar eclipse", format!("seen from {} under {}", p.name, mo.name)),
                        orbit_core::eclipse::Kind::Lunar => ("Lunar eclipse", format!("{} in the shadow of {}", mo.name, p.name)),
                    };
                    let wait = when - self.t;
                    self.chat.line(vec![
                        (format!("{what} in {}: ", duration(wait)), YELLOW),
                        (format!("{place}, {:.0}% of the sun hidden. ", deep * 100.0), WHITE),
                        (format!("/time add {:.0}", (wait - 10.0).max(0.0)), AQUA),
                    ]);
                }
                None => self.chat.info(&format!("No eclipse with {} in the next 6 hours", mo.name)),
            }
        }
    }

    fn rename(&mut self, name: &str) {
        let Some(i) = self.here() else { return };
        let mut s = self.stations.list[i].station.write().unwrap();
        s.name = orbit_core::station::clean_name(name);
        let n = s.name.clone();
        drop(s);
        self.chat.info(&format!("Renamed the station to {n}"));
    }

    /// Whether station `i` holds an elevator up.
    fn held_by_elevator(&self, i: usize) -> bool {
        let id = self.stations.list[i].station.read().unwrap().id.clone();
        self.elevators.list.iter().any(|s| s.e.station == id)
    }

    /// Brings out the hologram of an elevator to build on the ground (`elevator::run_placing`
    /// shows it and builds it on a right click), or puts it away.
    fn elevator_build(&mut self) {
        if self.elevators.placing.take().is_some() {
            self.bar.show("Elevator put away");
            return;
        }
        let b = &self.player.body;
        if b.gravity_source.is_none() || b.station.is_some() || b.cabin.is_some() {
            self.chat.error("Elevators are built from the ground: go down to a planet, near its equator, and /elevator build there");
            return;
        }
        self.elevators.placing = Some(Default::default());
        self.chat.info("Look where it should stand, on the equator: right click builds it there (its station goes up over it), left click puts it away");
    }

    /// The elevator of the station you are on or near, or the one whose ground stop is by you.
    fn elevator_here(&mut self) -> Option<usize> {
        let feet = self.player.body.world_point(self.player.body.pos);
        let by_ground = self.elevators.list.iter().position(|s| s.now.is_some_and(|n| n.to_world(n.dir * n.bottom as f32).distance(feet) < 300.0));
        let by_station = self.stations.near(&self.player.body, self.space, self.t, 300.0).and_then(|i| {
            let id = self.stations.list[i].station.read().unwrap().id.clone();
            self.elevators.list.iter().position(|s| s.e.station == id)
        });
        let riding = self.player.body.cabin.as_deref().and_then(|id| self.elevators.list.iter().position(|s| s.e.station == id));
        let found = riding.or(by_station).or(by_ground);
        if found.is_none() {
            self.chat.error("No elevator here: /elevator list");
        }
        found
    }

    fn elevator_info(&mut self) {
        let Some(k) = self.elevator_here() else { return };
        for l in elevator::describe(&self.elevators.list[k], self.space, self.stations) {
            self.chat.info(&l);
        }
    }

    fn elevator_list(&mut self) {
        if self.elevators.list.is_empty() {
            self.chat.line(vec![("No elevators yet: ".into(), WHITE), ("/elevator build".into(), AQUA)]);
            return;
        }
        for k in 0..self.elevators.list.len() {
            let e = &self.elevators.list[k].e;
            let name = self.stations.by_id(&e.station).map(|s| s.station.read().unwrap().name.clone()).unwrap_or_default();
            let line = format!("{} \"{name}\": from {}, the cabin {}", e.station, self.planet_name(e.planet), if e.at_top { "up" } else { "down" });
            self.chat.info(&line);
        }
    }

    fn elevator_remove(&mut self) {
        let Some(k) = self.elevator_here() else { return };
        if self.player.body.cabin.as_deref() == Some(self.elevators.list[k].e.station.as_str()) {
            self.chat.error("Step out of the cabin first");
            return;
        }
        let slot = self.elevators.list.remove(k);
        let name = self.stations.by_id(&slot.e.station).map(|s| s.station.read().unwrap().name.clone()).unwrap_or_default();
        self.bar.show("Elevator taken down");
        self.chat.info(&format!("Took down the elevator to {name}: the station keeps its synchronous orbit, now free to move"));
    }

    fn pack(&mut self) {
        let Some(i) = self.here() else { return };
        let (name, id) = {
            let s = self.stations.list[i].station.read().unwrap();
            (s.name.clone(), s.id.clone())
        };
        if self.held_by_elevator(i) {
            self.chat.line(vec![(format!("{name} holds a space elevator up. Take it down first with "), RED), ("/elevator remove".into(), AQUA)]);
            return;
        }
        self.stations.pack(i);
        self.bar.show(&format!("Packed {name}"));
        self.chat.line(vec![(format!("Packed {name}. Put it back up with "), WHITE), (format!("/station restore {id}"), AQUA)]);
    }

    fn restore(&mut self, id: &str) {
        let Some(i) = self.stations.list.iter().position(|s| s.station.read().unwrap().id == id) else {
            self.chat.error(&format!("No station {id}"));
            return;
        };
        if self.stations.list[i].station.read().unwrap().orbit.is_some() {
            let w = self.whereabouts(i);
            self.chat.error(&format!("Station {id} is placed {w}: pack it up from its core"));
            return;
        }
        match self.stations.deploy(Some(i), None, &self.player.body, self.space, self.t) {
            Ok(_) => {
                let s = self.stations.list[i].station.read().unwrap();
                let planet = self.planet_name(s.orbit.unwrap().planet);
                self.chat.info(&format!("Restored {} ({}) in orbit around {}", s.name, s.id, planet));
            }
            Err(r) => self.chat.error(r.message()),
        }
    }

    /// Puts the player on a station's deck, by its core, facing it.
    fn board(&mut self, i: usize) -> bool {
        let slot = &self.stations.list[i];
        let s = slot.station.read().unwrap();
        if s.orbit.is_none() {
            let id = s.id.clone();
            drop(s);
            self.chat.line(vec![(format!("Station {id} is packed: "), RED), (format!("/station restore {id}"), AQUA)]);
            return false;
        }
        let Some(f) = Stations::frame(&s, self.space, self.t) else {
            drop(s);
            self.chat.error("That station is too far away: fly or warp to its planet first");
            return false;
        };
        let deck = IVec3::new(0, 0, 2);
        let feet = if s.solid(deck) { deck.as_vec3() + Vec3::new(0.5, 1.0, 0.5) } else { Vec3::new(0.5, 1.0, 0.5) };
        self.travel.job = None;
        self.player.body.board(&f.at(&s), feet, Vec3::NEG_Z);
        let name = s.name.clone();
        drop(s);
        self.chat.info(&format!("Teleported {PLAYER} to {name}"));
        true
    }

    fn tp_station(&mut self, id: &str) {
        match self.stations.list.iter().position(|s| s.station.read().unwrap().id == id) {
            Some(i) => {
                self.board(i);
            }
            None => self.chat.error(&format!("No station {id}")),
        }
    }

    fn tp(&mut self, to: &str) {
        if let Some(i) = self.stations.find(to) {
            self.board(i);
            return;
        }
        let Some(p) = self.space.planets.values().find(|p| p.spec.name.eq_ignore_ascii_case(to)) else {
            self.chat.error(&format!("No station or planet called {to}"));
            return;
        };
        let spec = &p.spec;
        let body = &mut self.player.body;
        // Over where you are if it is the planet you are at; else over its pole.
        let up = if body.planet() == Some(spec.id) { body.point_from(body.pos, spec.center).normalize_or(Vec3::Y) } else { spec.rotation * Vec3::Y };
        let at = spec.center + (up * (spec.ceiling() - 4.0)).as_dvec3();
        self.travel.job = None;
        body.place(at, Vec3::ZERO, up, up.any_orthonormal_vector(), 0.0);
        self.chat.info(&format!("Teleported {PLAYER} to {}", spec.name));
    }

    /// The planet whose day it is for the player, and the spot on it (planet-local direction).
    fn day_place(&mut self) -> Option<(PlanetSpec, Vec3)> {
        let b = &self.player.body;
        let Some(p) = b.planet().and_then(|id| self.space.planets.get(&id)) else {
            self.chat.error("Not near a planet: out here there is no day or night");
            return None;
        };
        let local = p.spec.to_local(b.world_point(b.pos)).normalize_or(Vec3::Y);
        if p.spec.spin.speed.abs() < 1e-6 {
            self.chat.error(&format!("{} does not turn: its sun never moves", p.spec.name));
            return None;
        }
        Some((p.spec.clone(), local))
    }

    fn time_set(&mut self, what: &str) {
        if let Ok(v) = what.parse::<f64>() {
            self.shift.0 += v - self.t;
            self.chat.info(&format!("Set the time to {}", v.round() as i64));
            return;
        }
        let Some((spec, local)) = self.day_place() else { return };
        let day = (std::f64::consts::TAU / spec.spin.speed.abs() as f64).max(1.0);
        const N: usize = 720;
        let samples: Vec<(f64, f32)> = (0..=N).map(|i| {
            let t = self.t + day * i as f64 / N as f64;
            (t, sun_elevation(&spec, local, t))
        }).collect();
        let peak = |better: fn(f32, f32) -> bool| samples.iter().skip(1).fold(samples[1], |a, &b| if better(b.1, a.1) { b } else { a }).0;
        let crossing = |level: f32, rising: bool| {
            samples.windows(2).find(|w| if rising { w[0].1 < level && w[1].1 >= level } else { w[0].1 > level && w[1].1 <= level }).map(|w| w[1].0)
        };
        let noon = peak(|a, b| a > b);
        let midnight = peak(|a, b| a < b);
        let target = match what {
            "day" => crossing(12f32.to_radians(), true).unwrap_or(noon),
            "noon" => noon,
            "night" => crossing(-4f32.to_radians(), false).unwrap_or(midnight),
            _ => midnight,
        };
        self.shift.0 += target - self.t;
        let mut later = spec.clone();
        later.advance(target);
        self.chat.info(&format!("Set the time to {what}: {} on {}", clock_time(&later, local), spec.name));
    }

    fn time_query(&mut self, what: &str) {
        match what {
            "gametime" => self.chat.info(&format!("The time is {}", self.t as i64)),
            "day" => {
                let Some((spec, _)) = self.day_place() else { return };
                let day = std::f64::consts::TAU / spec.spin.speed.abs() as f64;
                self.chat.info(&format!("The time is {}", (self.t / day).floor() as i64));
            }
            _ => {
                let Some((spec, local)) = self.day_place() else { return };
                self.chat.info(&format!("The time is {} on {}", clock_time(&spec, local), spec.name));
            }
        }
    }
}

/// The sun's height over a spot's horizon (radians) at time `t`.
fn sun_elevation(spec: &PlanetSpec, local: Vec3, t: f64) -> f32 {
    let mut s = spec.clone();
    s.advance(t);
    let up = s.rotation * local;
    let sun = (s.star() - s.center).as_vec3().normalize_or(Vec3::Y);
    up.dot(sun).clamp(-1.0, 1.0).asin()
}

/// Local solar time at a spot, "HH:MM": noon when the sun is due over its meridian.
fn clock_time(spec: &PlanetSpec, local: Vec3) -> String {
    let axis = spec.spin.axis.normalize_or(Vec3::Y);
    let up = spec.rotation * local;
    let sun = (spec.star() - spec.center).as_vec3().normalize_or(Vec3::Y);
    let (a, b) = (up.reject_from(axis), sun.reject_from(axis));
    let angle = axis.dot(b.cross(a)).atan2(a.dot(b)) * spec.spin.speed.signum();
    let hours = (12.0 + angle / PI * 12.0).rem_euclid(24.0);
    let minutes = (hours * 60.0) as u32;
    format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Cmd, ParseError> {
        parse(&tree(), s).map(|p| p.cmd)
    }

    #[test]
    fn parses_commands() {
        assert_eq!(p("station place").unwrap(), Cmd::StationPlace);
        assert_eq!(p("station place My Base").unwrap(), Cmd::StationPlace);
        assert_eq!(parse(&tree(), "station place My Base").unwrap().args, vec!["My Base".to_string()]);
        assert_eq!(p("fourlokos station list").unwrap(), Cmd::StationList);
        assert_eq!(p("station height").unwrap(), Cmd::StationHeight);
        assert_eq!(p("station height 400").unwrap(), Cmd::StationHeight);
        assert_eq!(p("station height sync").unwrap(), Cmd::StationHeightSync);
        assert_eq!(p("schematic save Mi Base").unwrap(), Cmd::SchematicSave);
        assert_eq!(p("schematic place mi_base Copia").unwrap(), Cmd::SchematicPlace);
        assert_eq!(p("time set noon").unwrap(), Cmd::TimeSet);
        assert_eq!(p("time set 1200").unwrap(), Cmd::TimeSet);
        assert_eq!(p("help").unwrap(), Cmd::Help);
        assert_eq!(p("tp Kanis-42").unwrap(), Cmd::Tp);
    }

    #[test]
    fn errors_point_where_minecraft_does() {
        let e = p("foo").unwrap_err();
        assert_eq!((e.message.as_str(), e.at, e.incomplete), (UNKNOWN, 0, false));
        let e = p("station").unwrap_err();
        assert_eq!((e.message.as_str(), e.at, e.incomplete), (UNKNOWN, 7, true));
        let e = p("station fly").unwrap_err();
        assert_eq!((e.message.as_str(), e.at), (INCORRECT, 8));
        let e = p("station list now").unwrap_err();
        assert_eq!((e.message.as_str(), e.at), (INCORRECT, 13));
        let e = p("time add soon").unwrap_err();
        assert_eq!(e.message, "Invalid double 'soon'");
        assert!(p("station ").unwrap_err().incomplete);
    }

    #[test]
    fn usage_like_brigadier() {
        let t = tree();
        let station = t.iter().find(|n| n.literal() == Some("station")).unwrap();
        assert_eq!(smart(station), "(place|list|info|rename|pack|restore|tp|height)");
        let help = t.iter().find(|n| n.literal() == Some("help")).unwrap();
        assert_eq!(smart(help), "[<command>]");
        let mut all = Vec::new();
        all_usage(station, "", &mut all);
        assert!(all.contains(&"/station place".to_string()) && all.contains(&"/station place <name>".to_string()));
        assert!(all.contains(&"/station restore <id>".to_string()));
    }
}
