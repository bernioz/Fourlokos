use orbit_core::crafting;
use orbit_core::crop;
use orbit_core::dialogue::{Dialogue, Flags, Gift, Npc};
use orbit_core::furnace::{self, Furnace};
use orbit_core::inventory::Stack;
use orbit_core::{block, items};

fn grid(size: usize, entries: &[(usize, usize, block::Block)]) -> Vec<Option<block::Block>> {
    let mut cells = vec![None; size * size];
    for &(x, y, item) in entries {
        cells[y * size + x] = Some(item);
    }
    cells
}

fn smelt(input: block::Block) -> block::Block {
    let mut furnace = Furnace {
        input: Some(Stack::new(input, 1)),
        fuel: Some(Stack::new(items::COAL, 1)),
        ..Default::default()
    };
    furnace.tick(furnace::SMELT_TIME);
    furnace.output.expect("the recipe should smelt").item
}

#[test]
fn farm_to_completo_chain_uses_the_real_intermediate_items() {
    assert_eq!(crop::drop_for(block::SUNFLOWER_3), Some(items::SUNFLOWER_SEEDS));
    assert_eq!(smelt(items::SUNFLOWER_SEEDS), items::OIL);
    assert_eq!(smelt(items::RAW_CHICKEN), items::VIENESA);
    assert_eq!(crop::drop_for(block::WHEAT_3), Some(items::WHEAT));
    assert_eq!(crop::drop_for(block::PALTO_1), Some(items::AVOCADO));
    assert_eq!(crop::drop_for(block::TOMATO_3), Some(items::TOMATO));

    let mayonnaise = crafting::find(
        &grid(2, &[(0, 0, items::EGG), (1, 0, items::OIL)]),
        2,
    )
    .expect("egg and sunflower oil make mayonnaise");
    assert_eq!(mayonnaise.result, items::MAYONNAISE);

    let bread = crafting::find(
        &grid(3, &[(0, 0, items::WHEAT), (1, 0, items::WHEAT), (2, 0, items::WHEAT)]),
        3,
    )
    .expect("three wheat make bread");
    assert_eq!(bread.result, items::BREAD);

    let completo = crafting::find(
        &grid(
            3,
            &[
                (0, 0, bread.result),
                (1, 0, items::VIENESA),
                (2, 0, items::AVOCADO),
                (0, 1, items::TOMATO),
                (1, 1, mayonnaise.result),
            ],
        ),
        3,
    )
    .expect("the five finished ingredients make a completo");
    assert_eq!(completo.result, items::COMPLETO_ITALIANO);
}

#[test]
fn bodoque_requires_completo_and_dead_king_and_gives_kit_once() {
    let mut flags = Flags::default();
    flags.boss_defeated = true;
    let mut dialogue = Dialogue::new(Npc::Bodoque, &flags);
    assert_eq!(dialogue.node, 0);
    dialogue.advance(1, &mut flags);
    let missing = dialogue.advance_with_item(0, &mut flags, false).unwrap();
    assert_eq!(missing.gift, None);
    assert!(!flags.completo_delivered);

    let mut dialogue = Dialogue::new(Npc::Bodoque, &flags);
    dialogue.advance(1, &mut flags);
    let delivered = dialogue.advance_with_item(0, &mut flags, true).unwrap();
    assert_eq!(delivered.delivered, Some(items::COMPLETO_ITALIANO));
    assert!(flags.completo_delivered);

    let mut reward = Dialogue::new(Npc::Bodoque, &flags);
    assert_eq!(reward.node, 7);
    let gift = reward.advance(0, &mut flags).unwrap();
    assert_eq!(gift, Some(Gift::RocketKit));
    assert_eq!(Gift::RocketKit.item(), items::ROCKET_KIT);
    assert!(flags.rocket_kit_given());

    let mut again = Dialogue::new(Npc::Bodoque, &flags);
    assert_eq!(again.node, 8);
    assert_eq!(again.advance(0, &mut flags), Some(None));
    assert!(flags.rocket_kit_given());
}

#[test]
fn old_two_byte_flags_still_load_without_new_quest_state() {
    let flags = Flags::from_bytes(&[1, 1]).expect("old flags format should load");
    assert!(flags.gave(Npc::Bodoque));
    assert!(flags.boss_defeated);
    assert!(!flags.completo_delivered);
}
