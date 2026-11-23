//! What townsfolk say: a tree of nodes per NPC (a text and one to three replies), a little state
//! machine to walk it (`Dialogue`), and the per-world flags that remember one-time gifts and the
//! fallen Slime King. Pure data and logic: the client draws it and feeds the keys.

use crate::block::Block;
use crate::inventory::Stack;
use crate::items;
use crate::mob::Kind;

/// Most replies a node offers.
pub const MAX_CHOICES: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Npc {
    Bodoque,
    Tulio,
}

impl Npc {
    pub fn of(kind: Kind) -> Option<Npc> {
        match kind {
            Kind::Bodoque => Some(Npc::Bodoque),
            Kind::Tulio => Some(Npc::Tulio),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Npc::Bodoque => "Bodoque",
            Npc::Tulio => "Tulio",
        }
    }

    pub fn tree(self) -> &'static [Node] {
        match self {
            Npc::Bodoque => &BODOQUE,
            Npc::Tulio => &TULIO,
        }
    }

    /// The node a talk starts at: the first greeting, or the one for after the King's fall.
    pub fn start(self, flags: &Flags) -> usize {
        if self == Npc::Bodoque {
            if flags.rocket_kit_given() {
                8
            } else if flags.completo_delivered {
                if flags.boss_defeated { 7 } else { 6 }
            } else {
                0
            }
        } else if flags.boss_defeated {
            self.tree().len() - 1
        } else {
            0
        }
    }

    /// The flag bit of this NPC's one-time gift.
    fn gift_bit(self) -> u8 {
        match self {
            Npc::Bodoque => 1,
            Npc::Tulio => 2,
        }
    }
}

/// A one-time present.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gift {
    StonePickaxe,
    ThreeSlimeballs,
    RocketKit,
}

impl Gift {
    pub fn stack(self) -> Stack {
        match self {
            Gift::StonePickaxe => Stack::new(items::STONE_PICKAXE, 1),
            Gift::ThreeSlimeballs => Stack::new(items::SLIMEBALL, 3),
            Gift::RocketKit => Stack::new(items::ROCKET_KIT, 1),
        }
    }

    pub fn item(self) -> Block {
        self.stack().item
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    /// The gift, and the node to go to instead if it was already given.
    Gift(Gift, usize),
    /// An item is required; the first node is used after delivery and the second when missing.
    Deliver(Block, usize, usize),
}

#[derive(Clone, Copy, Debug)]
pub struct Choice {
    pub text: &'static str,
    /// The node it leads to; None ends the talk.
    pub next: Option<usize>,
    pub action: Action,
}

#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub text: &'static str,
    pub choices: &'static [Choice],
}

const fn go(text: &'static str, next: usize) -> Choice {
    Choice {
        text,
        next: Some(next),
        action: Action::None,
    }
}

const fn bye(text: &'static str) -> Choice {
    Choice {
        text,
        next: None,
        action: Action::None,
    }
}

const fn gift(text: &'static str, g: Gift, next: usize, taken: usize) -> Choice {
    Choice {
        text,
        next: Some(next),
        action: Action::Gift(g, taken),
    }
}

const fn deliver(text: &'static str, item: Block, next: usize, missing: usize) -> Choice {
    Choice {
        text,
        next: Some(next),
        action: Action::Deliver(item, next, missing),
    }
}

/// Bodoque, who looks after the meadow: how to get going, the moon, a warning about the King.
/// The last node is what he says once the King has fallen.
static BODOQUE: [Node; 9] = [
    Node {
        text: "¡Hola! Soy Bodoque. Este prado es mi hogar y me gusta ayudar a quienes llegan a Munia-18.",
        choices: &[
            go("¿Cómo empiezo?", 1),
            go("Tengo una misión para ti.", 2),
            go("¿Hay problemas por aquí?", 3),
        ],
    },
    Node {
        text: "Golpea un árbol para conseguir troncos, fabrica tablones, palos y una mesa. Con tablones haces un pico de madera; mantén el clic sobre la piedra. Ocho adoquines forman un horno para fundir minerales con carbón. Aquí tienes una ayuda.",
        choices: &[
            gift("Aceptar el pico de piedra", Gift::StonePickaxe, 0, 4),
            go("Volver", 0),
        ],
    },
    Node {
        text: "Necesito un completo italiano y que derrotes al Rey Slime. Si cumples ambas cosas, te entregaré algo que te ayudará a viajar más lejos.",
        choices: &[
            deliver(
                "Entregar el completo italiano",
                items::COMPLETO_ITALIANO,
                5,
                4,
            ),
            go("Volver", 0),
        ],
    },
    Node {
        text: "Los slimes saltan por el prado. Si no los molestas, no te perseguirán. Pero cuidado con el Rey Slime: derrotarlo es la otra parte de mi encargo.",
        choices: &[go("Volver", 0), bye("Entendido. Adiós.")],
    },
    Node {
        text: "Todavía no tienes el completo italiano. Consigue los ingredientes, prepáralo y vuelve a hablar conmigo.",
        choices: &[go("Volver", 0), bye("Volveré.")],
    },
    Node {
        text: "Gracias por el completo. El Rey Slime sigue vivo; cuando lo derrotes, vuelve y tendrás tu recompensa.",
        choices: &[
            go("Volver", 0),
            go("Ya derroté al Rey Slime", 7),
            go("Necesito más tiempo", 6),
        ],
    },
    Node {
        text: "El completo está listo. Derrota al Rey Slime y vuelve para recibir tu kit de cohete.",
        choices: &[go("Ya derroté al Rey Slime", 7), go("Volver", 0)],
    },
    Node {
        text: "¡Lo lograste! El completo está delicioso y el Rey Slime ha caído. Toma este kit de cohete; úsalo sobre el suelo para construir tu nave.",
        choices: &[
            gift("Aceptar el kit de cohete", Gift::RocketKit, 8, 8),
            go("Volver", 0),
        ],
    },
    Node {
        text: "El kit de cohete ya está en tus manos. Busca un terreno plano y despejado para construir tu nave.",
        choices: &[go("Volver", 0), bye("Gracias, Bodoque.")],
    },
];

/// Tulio, presentador de noticias de la luna: noticias, un consejo para llamar al Rey y un regalo.
static TULIO: [Node; 6] = [
    Node {
        text: "¡Buenas noches y bienvenidos a Noticias de Tulio! Soy Tulio, el presentador oficial. La noticia principal de hoy: ¡hay un visitante en mi luna!",
        choices: &[
            go("¿Qué dicen las noticias?", 1),
            go("¿Algún consejo?", 2),
            bye("Buenas noches, Tulio."),
        ],
    },
    Node {
        text: "Las noticias dicen que el Rey Slime duerme bajo Munia-18 y nadie se atreve a despertarlo. El pronóstico: polvo, con una ligera posibilidad de más polvo. ¡Volvemos contigo!",
        choices: &[go("¿Algún consejo?", 2), go("Volver", 0)],
    },
    Node {
        text: "Consejo exclusivo y gratuito: ocho bolas de slime alrededor de un lingote de hierro en una mesa fabrican un núcleo de corona de slime. Úsalo en el suelo y llegará el Rey. Lleva una espada. ¡Una buena; mejor varias!",
        choices: &[
            gift("¿Un regalo? ¡Claro!", Gift::ThreeSlimeballs, 3, 4),
            go("Volver", 0),
        ],
    },
    Node {
        text: "¡Última hora: un regalo de la audiencia! Tres bolas de slime recién salidas del estudio. No preguntes de dónde las saqué.",
        choices: &[go("Volver", 0), bye("¡Gracias, Tulio!")],
    },
    Node {
        text: "Esas eran todas las bolas de slime del estudio. ¡Sigue atento a más noticias y a menos regalos!",
        choices: &[go("Volver", 0), bye("Buenas noches, Tulio.")],
    },
    Node {
        text: "¡ÚLTIMA HORA: el Rey Slime ha caído! Fuentes cercanas al títere dicen que tú lo lograste. Noticias de Tulio te saluda, y también se felicita a sí mismo.",
        choices: &[go("¿Algo más?", 0), bye("Un día más de trabajo.")],
    },
];

/// What the world remembers about talks and the King, kept with the world.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Flags {
    pub given: u8,
    pub boss_defeated: bool,
    pub completo_delivered: bool,
}

const ROCKET_KIT_GIVEN: u8 = 4;

impl Flags {
    pub fn gave(&self, npc: Npc) -> bool {
        self.given & npc.gift_bit() != 0
    }

    pub fn rocket_kit_given(&self) -> bool {
        self.given & ROCKET_KIT_GIVEN != 0
    }

    pub fn to_bytes(self) -> [u8; 3] {
        [
            self.given,
            self.boss_defeated as u8,
            self.completo_delivered as u8,
        ]
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let &[given, boss, ..] = b else { return None };
        let completo = b.get(2).copied().unwrap_or(0);
        (given <= 7 && boss <= 1 && completo <= 1).then_some(Self {
            given,
            boss_defeated: boss == 1,
            completo_delivered: completo == 1,
        })
    }
}

/// A talk in progress.
#[derive(Clone, Copy, Debug)]
pub struct Dialogue {
    pub npc: Npc,
    pub node: usize,
    pub ended: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advance {
    pub gift: Option<Gift>,
    pub delivered: Option<Block>,
}

impl Dialogue {
    pub fn new(npc: Npc, flags: &Flags) -> Self {
        Self {
            npc,
            node: npc.start(flags),
            ended: false,
        }
    }

    pub fn node(&self) -> &'static Node {
        &self.npc.tree()[self.node]
    }

    pub fn choices(&self) -> &'static [Choice] {
        self.node().choices
    }

    /// Takes reply `i` (None if there is no such reply or the talk is over). Returns the gift
    /// handed over, if this reply gave one for the first time; it is flagged as given.
    pub fn advance(&mut self, i: usize, flags: &mut Flags) -> Option<Option<Gift>> {
        self.advance_with_item(i, flags, false)
            .map(|r| Some(r.gift).flatten())
    }

    pub fn advance_with_item(
        &mut self,
        i: usize,
        flags: &mut Flags,
        has_item: bool,
    ) -> Option<Advance> {
        if self.ended {
            return None;
        }
        let c = *self.choices().get(i)?;
        let mut given = None;
        let mut next = c.next;
        let mut delivered = None;
        if let Action::Gift(g, taken) = c.action {
            let already = match g {
                Gift::RocketKit => flags.rocket_kit_given(),
                _ => flags.gave(self.npc),
            };
            if already {
                next = Some(taken);
            } else {
                flags.given |= if g == Gift::RocketKit {
                    ROCKET_KIT_GIVEN
                } else {
                    self.npc.gift_bit()
                };
                given = Some(g);
            }
        } else if let Action::Deliver(item, success, missing) = c.action {
            if has_item {
                flags.completo_delivered = true;
                delivered = Some(item);
                next = Some(if flags.boss_defeated { 6 } else { success });
            } else {
                next = Some(missing);
            }
        }
        match next {
            Some(n) => self.node = n,
            None => self.ended = true,
        }
        Some(Advance {
            gift: given,
            delivered,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tree_is_well_formed_and_reachable() {
        for npc in [Npc::Bodoque, Npc::Tulio] {
            let tree = npc.tree();
            let mut seen = vec![false; tree.len()];
            let mut todo = vec![0, tree.len() - 1];
            while let Some(n) = todo.pop() {
                if std::mem::replace(&mut seen[n], true) {
                    continue;
                }
                let node = &tree[n];
                assert!(!node.text.is_empty(), "{npc:?} {n}");
                assert!(
                    (1..=MAX_CHOICES).contains(&node.choices.len()),
                    "{npc:?} node {n} has {} replies",
                    node.choices.len()
                );
                for c in node.choices {
                    assert!(!c.text.is_empty());
                    for t in c.next.into_iter().chain(match c.action {
                        Action::Gift(_, t) | Action::Deliver(_, t, _) => Some(t),
                        Action::None => None,
                    }) {
                        assert!(t < tree.len(), "{npc:?} node {n} leads to {t}");
                        todo.push(t);
                    }
                }
                // Every node leaves a way out.
                assert!(
                    node.choices.iter().any(|c| c.next.is_none())
                        || n == 0
                        || node.choices.iter().any(|c| c.next == Some(0))
                );
            }
            assert!(
                seen.iter().all(|&s| s),
                "{npc:?} has nodes nobody can reach: {seen:?}"
            );
        }
    }

    #[test]
    fn a_talk_advances_and_ends() {
        let mut flags = Flags::default();
        let mut d = Dialogue::new(Npc::Tulio, &flags);
        assert_eq!(d.node, 0);
        assert_eq!(d.choices().len(), 3);
        assert_eq!(d.advance(9, &mut flags), None, "no such reply");
        assert_eq!(d.advance(0, &mut flags), Some(None)); // the news
        assert_eq!(d.node, 1);
        assert_eq!(d.advance(1, &mut flags), Some(None)); // "Back"
        assert_eq!(d.node, 0);
        assert!(!d.ended);
        assert_eq!(d.advance(2, &mut flags), Some(None)); // goodbye
        assert!(d.ended);
        assert_eq!(
            d.advance(0, &mut flags),
            None,
            "a finished talk takes no more replies"
        );
    }

    #[test]
    fn a_gift_is_given_once_per_world() {
        let mut flags = Flags::default();
        let mut d = Dialogue::new(Npc::Bodoque, &flags);
        d.advance(0, &mut flags); // How do I get started?
        assert_eq!(d.node, 1);
        assert!(!flags.gave(Npc::Bodoque));
        assert_eq!(d.advance(0, &mut flags), Some(Some(Gift::StonePickaxe)));
        assert!(flags.gave(Npc::Bodoque) && !flags.gave(Npc::Tulio));
        assert_eq!(Gift::StonePickaxe.item(), items::STONE_PICKAXE);
        // Ask again: no second gift, and he says so.
        let mut d = Dialogue::new(Npc::Bodoque, &flags);
        d.advance(0, &mut flags);
        assert_eq!(d.advance(0, &mut flags), Some(None));
        assert_eq!(d.node, 4);
        // Tulio's is separate.
        let mut t = Dialogue::new(Npc::Tulio, &flags);
        t.advance(1, &mut flags); // advice
        assert_eq!(t.advance(0, &mut flags), Some(Some(Gift::ThreeSlimeballs)));
        assert_eq!(Gift::ThreeSlimeballs.stack().count, 3);
    }

    #[test]
    fn after_the_king_falls_they_talk_about_it() {
        let mut flags = Flags::default();
        assert_eq!(Dialogue::new(Npc::Bodoque, &flags).node, 0);
        flags.boss_defeated = true;
        assert_eq!(Dialogue::new(Npc::Bodoque, &flags).node, 0);
        assert_eq!(
            Dialogue::new(Npc::Tulio, &flags).node,
            Npc::Tulio.tree().len() - 1
        );
    }

    #[test]
    fn the_hints_are_there() {
        let all = |n: Npc| {
            n.tree()
                .iter()
                .map(|x| x.text)
                .collect::<Vec<_>>()
                .join(" ")
        };
        let b = all(Npc::Bodoque);
        assert!(
            b.contains("pico")
                && b.contains("horno")
                && b.contains("Rey Slime")
                && b.contains("completo italiano")
        );
        let t = all(Npc::Tulio);
        assert!(
            t.contains("núcleo de corona de slime")
                && t.contains("ocho bolas de slime")
                && t.contains("noticias")
        );
    }

    #[test]
    fn flags_round_trip() {
        let mut f = Flags::default();
        let mut d = Dialogue::new(Npc::Tulio, &f);
        d.advance(1, &mut f);
        d.advance(0, &mut f); // the gift
        f.boss_defeated = true;
        assert_eq!(Flags::from_bytes(&f.to_bytes()), Some(f));
        assert_eq!(Flags::from_bytes(&[9, 0]), None);
        assert_eq!(Flags::from_bytes(&[1]), None);
        assert_eq!(
            Flags::from_bytes(&[1, 0]),
            Some(Flags {
                given: 1,
                ..Flags::default()
            })
        );
        assert_eq!(
            Flags::from_bytes(&Flags::default().to_bytes()),
            Some(Flags::default())
        );
    }

    #[test]
    fn bodoque_delivers_the_rocket_kit_once_after_the_quest() {
        let mut flags = Flags::default();
        let mut d = Dialogue::new(Npc::Bodoque, &flags);
        d.advance(1, &mut flags);
        assert_eq!(
            d.advance_with_item(0, &mut flags, false).unwrap().delivered,
            None
        );
        assert!(!flags.completo_delivered && !flags.rocket_kit_given());
        let mut d = Dialogue::new(Npc::Bodoque, &flags);
        d.advance(1, &mut flags);
        assert_eq!(
            d.advance_with_item(0, &mut flags, true).unwrap().delivered,
            Some(items::COMPLETO_ITALIANO)
        );
        assert!(flags.completo_delivered);
        flags.boss_defeated = true;
        let mut d = Dialogue::new(Npc::Bodoque, &flags);
        assert_eq!(d.node, 7);
        assert_eq!(d.advance(0, &mut flags), Some(Some(Gift::RocketKit)));
        assert!(flags.rocket_kit_given());
        let mut d = Dialogue::new(Npc::Bodoque, &flags);
        assert_eq!(d.node, 8);
        assert_eq!(d.advance(0, &mut flags), Some(None));
    }

    #[test]
    fn npcs_come_from_kinds() {
        assert_eq!(Npc::of(Kind::Bodoque), Some(Npc::Bodoque));
        assert_eq!(Npc::of(Kind::Tulio).map(|n| n.name()), Some("Tulio"));
        assert_eq!(Npc::of(Kind::Slime), None);
    }
}
