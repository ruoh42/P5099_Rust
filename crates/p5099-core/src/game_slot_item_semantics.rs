use crate::game_slot_item_schema::{ItemOperationSchema, ValidatedItemOperation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemSemanticEvidence {
    Unresolved,
    StaticConsumer,
    ProducerAndConsumer,
    RetainedTraceCorrelated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemLifecycleMeaning {
    Unknown,
    Initialize,
    Place,
    Launch,
    Activate,
    Impact,
    Resolve,
    Retarget,
    Remove,
    RebindSource,
    Respawn,
    UpdateRuntimeFlag,
    NoClientAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemOperationSemantics {
    pub meaning: ItemLifecycleMeaning,
    pub native_phase: Option<u8>,
    pub source_object_id: Option<u32>,
    pub target_object_id: Option<u32>,
    pub target_object_ids: Option<ItemObjectIdList>,
    pub transition_token: Option<u32>,
    pub variant: Option<u8>,
    pub effect_item_id: Option<u16>,
    pub evidence: ItemSemanticEvidence,
}

impl ItemOperationSemantics {
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            meaning: ItemLifecycleMeaning::Unknown,
            native_phase: None,
            source_object_id: None,
            target_object_id: None,
            target_object_ids: None,
            transition_token: None,
            variant: None,
            effect_item_id: None,
            evidence: ItemSemanticEvidence::Unresolved,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemObjectIdList {
    raw_offset: usize,
    count: usize,
}

impl ItemObjectIdList {
    #[must_use]
    pub const fn raw_offset(self) -> usize {
        self.raw_offset
    }

    #[must_use]
    pub const fn count(self) -> usize {
        self.count
    }

    #[must_use]
    pub fn decode(self, raw: &[u8]) -> Option<Vec<u32>> {
        let byte_length = self.count.checked_mul(4)?;
        let end = self.raw_offset.checked_add(byte_length)?;
        Some(
            raw.get(self.raw_offset..end)?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|bytes| u32::from_le_bytes(*bytes))
                .collect(),
        )
    }
}

#[must_use]
pub(crate) fn decode_validated_item_operation_semantics(
    validated: ValidatedItemOperation,
    raw: &[u8],
) -> ItemOperationSemantics {
    decode_item_operation_semantics(validated.schema(), raw, validated.state())
}

fn decode_item_operation_semantics(
    schema: &'static ItemOperationSchema,
    raw: &[u8],
    state: u32,
) -> ItemOperationSemantics {
    match schema.class_name {
        "GopAngel" => angel(raw, state),
        "GopBalloon" => balloon(raw, state),
        "GopBanana" => banana(raw, state),
        "GopBarricade" => barricade(raw, state),
        "GopBigTimebomb" => with_effect_item_id(big_timebomb(raw, state), 122),
        "GopBlock" => block(raw, state),
        "GopBossPrison" => boss_prison(raw, state),
        "GopBoundRoad" => bound_road(raw, state),
        "GopBoundWall" => bound_wall(raw, state),
        "GopAreaUfo" => area_ufo(raw, state),
        "GopCokebomb" => bomb(raw, state, BombPhase4Source::Auxiliary),
        "GopCloud" | "GopCloud2" => cloud(schema.class_name, raw, state),
        "GopCourse" => course(raw),
        "GopCube" => cube(raw, state),
        "GopCubeForBoss" => cube_for_boss(raw, state),
        "GopDevil" => devil(raw, state, true, ItemSemanticEvidence::StaticConsumer),
        "GopDynamite" | "GopHammer" => targeted_phase_three_effect(raw, state),
        "GopEmp" => emp(raw, state),
        "GopEventObject" => event_object(raw),
        "GopFalling" => falling(raw, state),
        "GopForceZone" => force_zone(raw, state),
        "GopSnowbomb" => bomb(raw, state, BombPhase4Source::Target),
        "GopGoldShield" => gold_shield(raw, state),
        "GopCokeRocket" | "GopGoldRocket" => {
            rocket_variant(raw, state, RocketVariantFollowup::ThroughState10)
        }
        "GopDinoClawRocket" | "GopTigerRocket" => {
            rocket_variant(raw, state, RocketVariantFollowup::ThroughState9)
        }
        "GopInfectedBomb" => infected_bomb(raw, state),
        "GopIcefly" => with_effect_item_id(icefly(raw, state), 80),
        "GopGiantTalisman" | "GopWitchUnionMagic" => actor_bound_native_phase(raw, state),
        "GopHeadBand" => head_band(raw, state),
        "GopItemTimeFlybomb" | "GopTimeCokebomb" => time_coke_bomb(raw, state),
        "GopLockdownRocket" => lockdown_rocket(raw, state),
        "GopMagnet" => magnet(raw, state),
        "GopMine" => mine(raw, state),
        "GopMovingUfo" => moving_ufo(raw, state),
        "GopMqDevil" => devil(raw, state, true, ItemSemanticEvidence::ProducerAndConsumer),
        "GopNewDevil" => devil(raw, state, false, ItemSemanticEvidence::ProducerAndConsumer),
        "GopOil" => oil(raw, state),
        "GopPiratebomb" => pirate_bomb(raw, state),
        "GopPress" => press(raw, state),
        "GopRobotBeam" => spatial_target_effect(raw, state, 2),
        "GopRollingCokebomb" | "GopRollingbomb" => rolling_bomb(raw, state),
        "GopRollingInfectedbomb" => rolling_infected_bomb(raw, state),
        "GopRocket" => rocket(raw, state),
        "GopScanning" => scanning(raw, state),
        "GopShield" => shield(raw, state),
        "GopSilence" => silence(raw, state),
        "GopSiren" => siren(raw, state),
        "GopSirenShield" => siren_shield(raw, state),
        "GopSlotLock" => slot_lock(raw, state),
        "GopSnowman" => rocket_variant(raw, state, RocketVariantFollowup::ThroughState7),
        "GopSpecialShield" => with_effect_item_id(special_shield(raw, state), 40),
        "GopSpecialSiren" => special_siren(raw, state),
        "GopSpecialSmall" => special_small(raw, state),
        "GopSpaceCraft" => space_craft(raw, state),
        "GopSpeedDown" => speed_down(raw, state),
        "GopStraightRocket" => with_effect_item_id(straight_rocket(raw, state), 73),
        "GopInfectedWaterfly" | "GopSnowWaterfly" | "GopWaterfly" => {
            waterfly(schema.class_name, raw, state)
        }
        "GopSuperMag" => super_magnet(raw, state),
        "GopTargetKart" => target_kart(state),
        "GopTimeMine" => time_mine(raw, state),
        "GopTimeInfectedBomb" => time_infected_bomb(raw, state),
        "GopTimeSnowbomb" => time_snow_bomb(raw, state),
        "GopTimebomb" => with_effect_item_id(time_snow_bomb(raw, state), 13),
        "GopTombStone" => spatial_target_effect(raw, state, 1),
        "GopThunderbolt" => thunderbolt(raw, state),
        "GopUfo" => ufo(raw, state),
        "GopGhost" => ghost(raw, state),
        "GopWaterMine" => water_mine(raw, state),
        "GopWaterbomb" => waterbomb(raw, state),
        "GopWaterbombFly" => waterbomb_fly(raw, state),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn boss_prison(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(3),
            None,
            None,
            object_id(raw, 64),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Remove,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn bound_road(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 53),
            None,
            object_id(raw, 20),
            byte(raw, 62),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 | 3 => contact_hazard_followup(raw, state, 2, 5, None),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn falling(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 85),
            None,
            object_id(raw, 20),
            byte(raw, 90),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 | 3 => contact_hazard_followup(raw, state, 3, 5, Some(29)),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn contact_hazard_followup(
    raw: &[u8],
    state: u32,
    state_two_phase: u8,
    state_three_phase: u8,
    source_offset: Option<usize>,
) -> ItemOperationSemantics {
    let has_target = byte(raw, 28).is_some_and(|flag| flag != 0);
    let meaning = if !has_target {
        ItemLifecycleMeaning::Remove
    } else if state == 2 {
        ItemLifecycleMeaning::Impact
    } else {
        ItemLifecycleMeaning::Resolve
    };
    let native_phase = has_target.then_some(if state == 2 {
        state_two_phase
    } else {
        state_three_phase
    });
    semantic(
        meaning,
        native_phase,
        has_target
            .then(|| source_offset.and_then(|offset| object_id(raw, offset)))
            .flatten(),
        has_target.then(|| object_id(raw, 24)).flatten(),
        object_id(raw, 20),
        byte(raw, 28),
        ItemSemanticEvidence::ProducerAndConsumer,
    )
}

fn course(raw: &[u8]) -> ItemOperationSemantics {
    semantic(
        ItemLifecycleMeaning::NoClientAction,
        None,
        None,
        object_id(raw, 12),
        course_transition_token(raw),
        None,
        ItemSemanticEvidence::ProducerAndConsumer,
    )
}

fn course_transition_token(raw: &[u8]) -> Option<u32> {
    let code_units = usize::try_from(u32_at(raw, 16)?).ok()?;
    let string_bytes = code_units.checked_mul(2)?;
    let token_offset = 20usize.checked_add(string_bytes)?;
    u32_at(raw, token_offset)
}

fn pirate_bomb(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            None,
            object_id(raw, 24),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Remove,
            Some(3),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(4),
            object_id(raw, 24),
            object_id(raw, 24),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn balloon(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(1),
            None,
            None,
            object_id(raw, 16),
            byte(raw, 20),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::UpdateRuntimeFlag,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn head_band(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(1),
            None,
            None,
            object_id(raw, 16),
            byte(raw, 20),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::UpdateRuntimeFlag,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn targeted_phase_three_effect(raw: &[u8], state: u32) -> ItemOperationSemantics {
    let (meaning, native_phase) = match state {
        1 => (ItemLifecycleMeaning::Activate, 0),
        2 => (ItemLifecycleMeaning::Impact, 3),
        _ => return ItemOperationSemantics::unknown(),
    };
    semantic(
        meaning,
        Some(native_phase),
        object_id(raw, 20),
        object_id(raw, 24),
        object_id(raw, 16),
        byte(raw, 28),
        ItemSemanticEvidence::ProducerAndConsumer,
    )
}

fn press(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(5),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn spatial_target_effect(
    raw: &[u8],
    state: u32,
    impact_native_phase: u8,
) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Unknown,
            Some(impact_native_phase),
            None,
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn actor_bound_native_phase(raw: &[u8], native_phase: u32) -> ItemOperationSemantics {
    semantic(
        ItemLifecycleMeaning::Unknown,
        u8::try_from(native_phase).ok(),
        object_id(raw, 24),
        object_id(raw, 20),
        object_id(raw, 16),
        None,
        ItemSemanticEvidence::ProducerAndConsumer,
    )
}

fn event_object(raw: &[u8]) -> ItemOperationSemantics {
    semantic(
        ItemLifecycleMeaning::Unknown,
        None,
        None,
        object_id(raw, 12),
        object_id(raw, 16),
        None,
        ItemSemanticEvidence::ProducerAndConsumer,
    )
}

fn target_kart(state: u32) -> ItemOperationSemantics {
    if state == 2 {
        semantic(
            ItemLifecycleMeaning::NoClientAction,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        )
    } else {
        ItemOperationSemantics::unknown()
    }
}

fn block(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 24),
            None,
            object_id(raw, 20),
            byte(raw, 88),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => {
            let hit = byte(raw, 28).is_some_and(|flag| flag != 0);
            semantic(
                if hit {
                    ItemLifecycleMeaning::Impact
                } else {
                    ItemLifecycleMeaning::Resolve
                },
                Some(if hit { 3 } else { 4 }),
                None,
                hit.then(|| object_id(raw, 24)).flatten(),
                object_id(raw, 20),
                byte(raw, 28),
                ItemSemanticEvidence::ProducerAndConsumer,
            )
        }
        _ => ItemOperationSemantics::unknown(),
    }
}

fn bound_wall(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 24),
            None,
            object_id(raw, 20),
            byte(raw, 136),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 | 3 => {
            let has_target = byte(raw, 28).is_some_and(|flag| flag != 0);
            semantic(
                if has_target {
                    if state == 2 {
                        ItemLifecycleMeaning::Impact
                    } else {
                        ItemLifecycleMeaning::Resolve
                    }
                } else {
                    ItemLifecycleMeaning::Remove
                },
                has_target.then(|| u8::try_from(state).ok()).flatten(),
                None,
                has_target.then(|| object_id(raw, 24)).flatten(),
                object_id(raw, 20),
                byte(raw, 28),
                ItemSemanticEvidence::ProducerAndConsumer,
            )
        }
        _ => ItemOperationSemantics::unknown(),
    }
}

fn cube(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Impact,
            None,
            None,
            object_id(raw, 16),
            object_id(raw, 20),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::NoClientAction,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::StaticConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn cube_for_boss(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        0 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            None,
            None,
            None,
            byte(raw, 68),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        1 => semantic(
            ItemLifecycleMeaning::Impact,
            None,
            None,
            object_id(raw, 69),
            object_id(raw, 73),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn angel(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        0 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 21),
            None,
            object_id(raw, 16),
            byte(raw, 20),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn gold_shield(raw: &[u8], state: u32) -> ItemOperationSemantics {
    let kind_offset = if state == 0 { 24 } else { 28 };
    let Some(kind) = u32_at(raw, kind_offset) else {
        return ItemOperationSemantics::unknown();
    };
    let effect_item_id = if state == 2 && u16_at(raw, 32) == Some(106) {
        106
    } else {
        match kind {
            0 => 36,
            3 => 81,
            _ => return ItemOperationSemantics::unknown(),
        }
    };
    let mut decoded = match state {
        0 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            u8::try_from(kind).ok(),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            u8::try_from(kind).ok(),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => return ItemOperationSemantics::unknown(),
    };
    decoded.effect_item_id = Some(effect_item_id);
    decoded
}

fn emp(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        0 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 22),
            None,
            object_id(raw, 16),
            byte(raw, 20),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn icefly(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            byte(raw, 77),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn scanning(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 16),
            object_id(raw, 24),
            object_id(raw, 20),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn slot_lock(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 16),
            None,
            object_id(raw, 24),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(1),
            object_id(raw, 16),
            object_id(raw, 20),
            object_id(raw, 24),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn ghost(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn special_siren(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        0 => semantic(
            ItemLifecycleMeaning::Activate,
            None,
            object_id(raw, 21),
            None,
            object_id(raw, 16),
            byte(raw, 20),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn space_craft(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        0 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 29),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(3),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::StaticConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(6),
            None,
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        5 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(5),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        7 => semantic(
            ItemLifecycleMeaning::UpdateRuntimeFlag,
            None,
            None,
            None,
            None,
            byte(raw, 16),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn straight_rocket(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(1),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            byte(raw, 56),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 | 3 => explicit_no_client_action(),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn cloud(class_name: &str, raw: &[u8], state: u32) -> ItemOperationSemantics {
    let mut decoded = match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            byte(raw, 24),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            None,
            object_id(raw, 16),
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    };
    if state == 1 {
        decoded.effect_item_id = match (class_name, decoded.variant) {
            ("GopCloud", Some(0)) => Some(0),
            ("GopCloud", Some(3)) => Some(1),
            ("GopCloud", Some(6)) => Some(43),
            ("GopCloud2", Some(0)) => Some(114),
            ("GopCloud2", Some(3)) => Some(115),
            ("GopCloud2", Some(6)) => Some(116),
            _ => None,
        };
    }
    decoded
}

fn magnet(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(1),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn speed_down(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            None,
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::StaticConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Remove,
            Some(2),
            None,
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::StaticConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn devil(
    raw: &[u8],
    state: u32,
    has_conditional_target: bool,
    evidence: ItemSemanticEvidence,
) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 21),
            (has_conditional_target && byte(raw, 20) == Some(5))
                .then(|| object_id(raw, 27))
                .flatten(),
            object_id(raw, 16),
            byte(raw, 20),
            evidence,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn force_zone(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 68),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => {
            let succeeded = byte(raw, 24).is_some_and(|flag| flag != 0);
            semantic(
                if succeeded {
                    ItemLifecycleMeaning::Impact
                } else {
                    ItemLifecycleMeaning::Resolve
                },
                succeeded.then_some(2),
                object_id(raw, 25),
                succeeded.then(|| object_id(raw, 20)).flatten(),
                object_id(raw, 16),
                byte(raw, 24),
                ItemSemanticEvidence::ProducerAndConsumer,
            )
        }
        3 => {
            let succeeded = byte(raw, 24).is_some_and(|flag| flag != 0);
            semantic(
                ItemLifecycleMeaning::Resolve,
                succeeded.then_some(3),
                succeeded.then(|| object_id(raw, 20)).flatten(),
                succeeded.then(|| object_id(raw, 25)).flatten(),
                object_id(raw, 16),
                byte(raw, 24),
                ItemSemanticEvidence::ProducerAndConsumer,
            )
        }
        5 => {
            let succeeded = byte(raw, 24).is_some_and(|flag| flag != 0);
            semantic(
                ItemLifecycleMeaning::Resolve,
                succeeded.then_some(5),
                succeeded.then(|| object_id(raw, 20)).flatten(),
                None,
                None,
                byte(raw, 24),
                ItemSemanticEvidence::ProducerAndConsumer,
            )
        }
        _ => ItemOperationSemantics::unknown(),
    }
}

fn oil(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 69),
            None,
            object_id(raw, 16),
            byte(raw, 20),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => {
            let succeeded = byte(raw, 24).is_some_and(|flag| flag != 0);
            semantic(
                if succeeded {
                    ItemLifecycleMeaning::Impact
                } else {
                    ItemLifecycleMeaning::Remove
                },
                succeeded.then_some(2),
                object_id(raw, 25),
                succeeded.then(|| object_id(raw, 20)).flatten(),
                object_id(raw, 16),
                byte(raw, 24),
                ItemSemanticEvidence::ProducerAndConsumer,
            )
        }
        3 => {
            let succeeded = byte(raw, 24).is_some_and(|flag| flag != 0);
            semantic(
                if succeeded {
                    ItemLifecycleMeaning::Resolve
                } else {
                    ItemLifecycleMeaning::Remove
                },
                succeeded.then_some(3),
                succeeded.then(|| object_id(raw, 20)).flatten(),
                None,
                object_id(raw, 16),
                byte(raw, 24),
                ItemSemanticEvidence::StaticConsumer,
            )
        }
        _ => ItemOperationSemantics::unknown(),
    }
}

fn silence(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 21),
            object_id(raw, 25),
            object_id(raw, 17),
            byte(raw, 16),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::NoClientAction,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::StaticConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn siren(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 21),
            None,
            object_id(raw, 16),
            byte(raw, 20),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(1),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn siren_shield(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        0 | 2 => semantic(
            if state == 0 {
                ItemLifecycleMeaning::Activate
            } else {
                ItemLifecycleMeaning::Resolve
            },
            u8::try_from(state).ok(),
            object_id(raw, 21),
            None,
            object_id(raw, 16),
            byte(raw, 20),
            if state == 0 {
                ItemSemanticEvidence::ProducerAndConsumer
            } else {
                ItemSemanticEvidence::StaticConsumer
            },
        ),
        1 => {
            let actor = object_id(raw, 20);
            semantic(
                ItemLifecycleMeaning::Impact,
                Some(1),
                actor,
                actor,
                object_id(raw, 16),
                None,
                ItemSemanticEvidence::StaticConsumer,
            )
        }
        _ => ItemOperationSemantics::unknown(),
    }
}

fn special_small(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        0 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 29),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        1 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(3),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::UpdateRuntimeFlag,
            None,
            None,
            None,
            None,
            byte(raw, 16),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RocketVariantFollowup {
    ThroughState7,
    ThroughState9,
    ThroughState10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BombPhase4Source {
    Auxiliary,
    Target,
}

const fn semantic(
    meaning: ItemLifecycleMeaning,
    native_phase: Option<u8>,
    source_object_id: Option<u32>,
    target_object_id: Option<u32>,
    transition_token: Option<u32>,
    variant: Option<u8>,
    evidence: ItemSemanticEvidence,
) -> ItemOperationSemantics {
    ItemOperationSemantics {
        meaning,
        native_phase,
        source_object_id,
        target_object_id,
        target_object_ids: None,
        transition_token,
        variant,
        effect_item_id: None,
        evidence,
    }
}

fn barricade(raw: &[u8], state: u32) -> ItemOperationSemantics {
    let evidence = if state == 2 {
        ItemSemanticEvidence::RetainedTraceCorrelated
    } else {
        ItemSemanticEvidence::ProducerAndConsumer
    };
    let (meaning, native_phase, variant) = match state {
        0 => (ItemLifecycleMeaning::Initialize, Some(0), None),
        1 => (ItemLifecycleMeaning::Place, Some(1), None),
        2 => (ItemLifecycleMeaning::Impact, Some(3), None),
        3 => (ItemLifecycleMeaning::Resolve, Some(4), None),
        4 => (ItemLifecycleMeaning::Remove, None, byte(raw, 25)),
        _ => return ItemOperationSemantics::unknown(),
    };
    semantic(
        meaning,
        native_phase,
        object_id(raw, 17),
        object_id(raw, 21),
        object_id(raw, 13),
        variant,
        evidence,
    )
}

fn banana(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 69),
            None,
            object_id(raw, 16),
            byte(raw, 20),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            None,
            object_id(raw, 25),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 24),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            None,
            object_id(raw, 20),
            object_id(raw, 25),
            object_id(raw, 16),
            byte(raw, 24),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn big_timebomb(raw: &[u8], native_phase: u32) -> ItemOperationSemantics {
    let source = object_id(raw, 25);
    let target = object_id(raw, 21);
    let Some((source, target)) = source.zip(target) else {
        return ItemOperationSemantics {
            meaning: ItemLifecycleMeaning::Unknown,
            evidence: ItemSemanticEvidence::ProducerAndConsumer,
            ..ItemOperationSemantics::unknown()
        };
    };

    let meaning = match native_phase {
        0 => ItemLifecycleMeaning::Activate,
        2 | 3 => ItemLifecycleMeaning::Impact,
        4 => ItemLifecycleMeaning::Resolve,
        _ => ItemLifecycleMeaning::Unknown,
    };
    semantic(
        meaning,
        u8::try_from(native_phase).ok(),
        Some(source),
        Some(target),
        object_id(raw, 17),
        byte(raw, 12),
        ItemSemanticEvidence::ProducerAndConsumer,
    )
}

fn area_ufo(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 25),
            object_id(raw, 21),
            object_id(raw, 16),
            byte(raw, 20),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => {
            let actor = object_id(raw, 20);
            semantic(
                ItemLifecycleMeaning::Resolve,
                Some(5),
                actor,
                actor,
                object_id(raw, 16),
                None,
                ItemSemanticEvidence::ProducerAndConsumer,
            )
        }
        _ => ItemOperationSemantics::unknown(),
    }
}

fn lockdown_rocket(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            None,
            None,
            object_id(raw, 14),
            byte(raw, 13),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Retarget,
            None,
            None,
            object_id(raw, 13),
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Remove,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::StaticConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(1),
            object_id(raw, 17),
            object_id(raw, 21),
            object_id(raw, 13),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        5 | 6 => semantic(
            ItemLifecycleMeaning::Resolve,
            None,
            None,
            None,
            object_id(raw, 13),
            byte(raw, 17),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        7 => semantic(
            ItemLifecycleMeaning::Resolve,
            byte(raw, 25).map(|variant| if variant == 0 { 7 } else { 8 }),
            object_id(raw, 17),
            object_id(raw, 21),
            object_id(raw, 13),
            byte(raw, 25),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        8 => semantic(
            ItemLifecycleMeaning::Resolve,
            byte(raw, 25).map(|variant| if variant == 0 { 11 } else { 10 }),
            object_id(raw, 17),
            object_id(raw, 21),
            object_id(raw, 13),
            byte(raw, 25),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        9 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(9),
            object_id(raw, 17),
            object_id(raw, 21),
            object_id(raw, 13),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn shield(raw: &[u8], state: u32) -> ItemOperationSemantics {
    let mut decoded = match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 22),
            None,
            object_id(raw, 18),
            byte(raw, 30),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(1),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    };
    if state == 1 {
        decoded.effect_item_id = u16_at(raw, 16);
    }
    decoded
}

fn special_shield(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        0 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 22),
            None,
            object_id(raw, 17),
            byte(raw, 16),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 | 3 => semantic(
            if state == 2 {
                ItemLifecycleMeaning::Impact
            } else {
                ItemLifecycleMeaning::Resolve
            },
            u8::try_from(state).ok(),
            object_id(raw, 21),
            None,
            object_id(raw, 17),
            byte(raw, 16),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn moving_ufo(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            None,
            None,
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn ufo(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 25),
            object_id(raw, 21),
            object_id(raw, 16),
            byte(raw, 20),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Resolve,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn thunderbolt(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => {
            let mut decoded = semantic(
                ItemLifecycleMeaning::Activate,
                Some(0),
                object_id(raw, 21),
                None,
                object_id(raw, 16),
                raw.last().copied(),
                ItemSemanticEvidence::ProducerAndConsumer,
            );
            decoded.target_object_ids = object_id_list(raw, 25, 29);
            decoded
        }
        2 | 3 => {
            let target = object_id(raw, 21);
            if target.is_none() {
                return semantic(
                    ItemLifecycleMeaning::Impact,
                    None,
                    None,
                    None,
                    None,
                    None,
                    ItemSemanticEvidence::ProducerAndConsumer,
                );
            }
            semantic(
                ItemLifecycleMeaning::Impact,
                Some(if state == 2 { 4 } else { 3 }),
                (state == 3).then(|| object_id(raw, 25)).flatten(),
                target,
                object_id(raw, 17),
                None,
                ItemSemanticEvidence::ProducerAndConsumer,
            )
        }
        _ => ItemOperationSemantics::unknown(),
    }
}

fn bomb(raw: &[u8], state: u32, phase4_source: BombPhase4Source) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(3),
            object_id(raw, 20),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(4),
            object_id(
                raw,
                match phase4_source {
                    BombPhase4Source::Auxiliary => 24,
                    BombPhase4Source::Target => 20,
                },
            ),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn infected_bomb(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            byte(raw, 120),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 29),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(4),
            object_id(raw, 29),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn rolling_bomb(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(3),
            object_id(raw, 20),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(4),
            object_id(raw, 20),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn rolling_infected_bomb(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 28),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(4),
            None,
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn time_coke_bomb(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 | 4 => semantic(
            ItemLifecycleMeaning::Resolve,
            u8::try_from(state).ok(),
            object_id(raw, 20),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn time_infected_bomb(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 28),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(4),
            None,
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn time_snow_bomb(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 | 4 => semantic(
            ItemLifecycleMeaning::Resolve,
            u8::try_from(state).ok(),
            object_id(raw, 20),
            object_id(raw, 20),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn mine(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 73),
            None,
            object_id(raw, 16),
            byte(raw, 72),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            None,
            object_id(raw, 25),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 24),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::NoClientAction,
            None,
            None,
            None,
            object_id(raw, 16),
            byte(raw, 24),
            ItemSemanticEvidence::StaticConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(4),
            object_id(raw, 20),
            object_id(raw, 25),
            object_id(raw, 16),
            byte(raw, 24),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        5 => semantic(
            ItemLifecycleMeaning::Remove,
            None,
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            byte(raw, 24),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        6 => semantic(
            ItemLifecycleMeaning::Respawn,
            Some(0),
            None,
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn water_mine(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 68),
            None,
            object_id(raw, 16),
            byte(raw, 72),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(3),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(4),
            object_id(raw, 24),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        7 => explicit_no_client_action(),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn time_mine(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Place,
            Some(0),
            object_id(raw, 81),
            None,
            object_id(raw, 20),
            byte(raw, 80),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => {
            let has_target_branch = byte(raw, 28).is_some_and(|flag| flag != 0);
            semantic(
                if has_target_branch {
                    ItemLifecycleMeaning::Impact
                } else {
                    ItemLifecycleMeaning::Resolve
                },
                None,
                None,
                has_target_branch.then(|| object_id(raw, 24)).flatten(),
                object_id(raw, 20),
                byte(raw, 28),
                ItemSemanticEvidence::ProducerAndConsumer,
            )
        }
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            None,
            None,
            byte(raw, 28)
                .is_some_and(|flag| flag != 0)
                .then(|| object_id(raw, 24))
                .flatten(),
            object_id(raw, 20),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => explicit_no_client_action(),
        5 => semantic(
            ItemLifecycleMeaning::Resolve,
            None,
            None,
            None,
            object_id(raw, 20),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn rocket(raw: &[u8], state: u32) -> ItemOperationSemantics {
    let mut decoded = match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(1),
            object_id(raw, 22),
            object_id(raw, 75),
            object_id(raw, 18),
            byte(raw, 26),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 68),
            None,
            object_id(raw, 16),
            byte(raw, 72),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(3),
            None,
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Retarget,
            None,
            None,
            object_id(raw, 16),
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        5 => semantic(
            ItemLifecycleMeaning::Remove,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        6 => semantic(
            ItemLifecycleMeaning::NoClientAction,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::StaticConsumer,
        ),
        7 => semantic(
            ItemLifecycleMeaning::RebindSource,
            Some(6),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        8..=10 => semantic(
            ItemLifecycleMeaning::Resolve,
            u8::try_from(state - 1).ok(),
            None,
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    };
    if state == 1 {
        decoded.effect_item_id = u16_at(raw, 16);
    }
    decoded
}

fn rocket_variant(
    raw: &[u8],
    state: u32,
    followup: RocketVariantFollowup,
) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(1),
            object_id(raw, 20),
            object_id(raw, 73),
            object_id(raw, 16),
            byte(raw, 24),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            object_id(raw, 68),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(3),
            None,
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Retarget,
            None,
            None,
            object_id(raw, 16),
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        5 => semantic(
            ItemLifecycleMeaning::Remove,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        7 => semantic(
            ItemLifecycleMeaning::RebindSource,
            Some(6),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        8 if followup != RocketVariantFollowup::ThroughState7 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(7),
            None,
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        9 if followup != RocketVariantFollowup::ThroughState7 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(8),
            None,
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        10 if followup == RocketVariantFollowup::ThroughState10 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(9),
            None,
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        6 | 8..=10 => explicit_no_client_action(),
        _ => ItemOperationSemantics::unknown(),
    }
}

const fn explicit_no_client_action() -> ItemOperationSemantics {
    semantic(
        ItemLifecycleMeaning::NoClientAction,
        None,
        None,
        None,
        None,
        None,
        ItemSemanticEvidence::StaticConsumer,
    )
}

fn super_magnet(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Activate,
            Some(0),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(4),
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::StaticConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn waterbomb(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            None,
            object_id(raw, 16),
            byte(raw, 120),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 | 3 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(if state == 2 { 2 } else { 3 }),
            object_id(raw, 25),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 24),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Remove,
            Some(4),
            object_id(raw, 25),
            object_id(raw, 20),
            object_id(raw, 16),
            byte(raw, 24),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn waterbomb_fly(raw: &[u8], state: u32) -> ItemOperationSemantics {
    match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(3),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(4),
            object_id(raw, 20),
            object_id(raw, 24),
            None,
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        5 => explicit_no_client_action(),
        6 => semantic(
            ItemLifecycleMeaning::Remove,
            None,
            None,
            None,
            None,
            None,
            ItemSemanticEvidence::StaticConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    }
}

fn waterfly(class_name: &str, raw: &[u8], state: u32) -> ItemOperationSemantics {
    let decoded = match state {
        1 => semantic(
            ItemLifecycleMeaning::Launch,
            Some(0),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            byte(raw, 28),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        2 => semantic(
            ItemLifecycleMeaning::Impact,
            None,
            None,
            None,
            None,
            byte(raw, 64),
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        3 => semantic(
            ItemLifecycleMeaning::Impact,
            Some(2),
            None,
            None,
            object_id(raw, 64),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        4 => semantic(
            ItemLifecycleMeaning::Resolve,
            Some(3),
            object_id(raw, 20),
            object_id(raw, 24),
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        5 => semantic(
            ItemLifecycleMeaning::Remove,
            Some(4),
            None,
            None,
            object_id(raw, 16),
            None,
            ItemSemanticEvidence::ProducerAndConsumer,
        ),
        _ => ItemOperationSemantics::unknown(),
    };
    match class_name {
        "GopWaterfly" => with_effect_item_id(decoded, 4),
        "GopSnowWaterfly" => with_effect_item_id(decoded, 118),
        "GopInfectedWaterfly" => with_effect_item_id(decoded, 119),
        _ => decoded,
    }
}

fn object_id(raw: &[u8], offset: usize) -> Option<u32> {
    let value = u32_at(raw, offset)?;
    (value != u32::MAX).then_some(value)
}

fn object_id_list(raw: &[u8], count_offset: usize, raw_offset: usize) -> Option<ItemObjectIdList> {
    let count = usize::try_from(u32_at(raw, count_offset)?).ok()?;
    let byte_length = count.checked_mul(4)?;
    let end = raw_offset.checked_add(byte_length)?;
    raw.get(raw_offset..end)?;
    Some(ItemObjectIdList { raw_offset, count })
}

fn u32_at(raw: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        raw.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn u16_at(raw: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        raw.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn byte(raw: &[u8], offset: usize) -> Option<u8> {
    raw.get(offset).copied()
}

const fn with_effect_item_id(
    mut semantics: ItemOperationSemantics,
    effect_item_id: u16,
) -> ItemOperationSemantics {
    semantics.effect_item_id = Some(effect_item_id);
    semantics
}

#[cfg(test)]
mod tests {
    use crate::game_slot_item_schema::item_operation_schema;

    use super::{ItemLifecycleMeaning, ItemSemanticEvidence, decode_item_operation_semantics};

    fn schema(pair: (u32, u32)) -> &'static crate::game_slot_item_schema::ItemOperationSchema {
        item_operation_schema(pair.0, pair.1).expect("test pair has a checked-in schema")
    }

    fn put_u32(raw: &mut [u8], offset: usize, value: u32) {
        raw[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn recovered_item_selectors_keep_exact_class_and_byte_meanings() {
        let shield = schema((0x1110_037F, 0x1D33_049E));
        let mut raw = vec![0; 31];
        raw[16..18].copy_from_slice(&18_u16.to_le_bytes());
        put_u32(&mut raw, 18, 0x7100_0001);
        put_u32(&mut raw, 22, 0x7200_0002);
        raw[30] = 9;
        let decoded = decode_item_operation_semantics(shield, &raw, 1);
        assert_eq!(decoded.effect_item_id, Some(18));
        assert_eq!(decoded.transition_token, Some(0x7100_0001));
        assert_eq!(decoded.source_object_id, Some(0x7200_0002));
        assert_eq!(decoded.variant, Some(9));

        let rocket = schema((0x1129_038E, 0x1D4C_04AD));
        let mut raw = vec![0; 82];
        raw[16..18].copy_from_slice(&33_u16.to_le_bytes());
        put_u32(&mut raw, 18, 0x7300_0003);
        let decoded = decode_item_operation_semantics(rocket, &raw, 1);
        assert_eq!(decoded.effect_item_id, Some(33));
        assert_eq!(decoded.transition_token, Some(0x7300_0003));

        for (pair, discriminator, item_id) in [
            ((0x0D7B_031D, 0x187F_043C), 0, 0),
            ((0x0D7B_031D, 0x187F_043C), 3, 1),
            ((0x0D7B_031D, 0x187F_043C), 6, 43),
            ((0x10CA_034F, 0x1CED_046E), 0, 114),
            ((0x10CA_034F, 0x1CED_046E), 3, 115),
            ((0x10CA_034F, 0x1CED_046E), 6, 116),
        ] {
            let mut raw = vec![0; 73];
            raw[24] = discriminator;
            let decoded = decode_item_operation_semantics(schema(pair), &raw, 1);
            assert_eq!(decoded.effect_item_id, Some(item_id));
        }

        for (pair, state, length, item_id) in [
            ((0x276B_0567, 0x3929_0686), 0, 29, 122),
            ((0x10C3_0382, 0x1CE6_04A1), 1, 78, 80),
            ((0x3473_0640, 0x486F_075F), 0, 27, 40),
            ((0x3C6F_06D4, 0x518A_07F3), 1, 58, 73),
            ((0x196A_0455, 0x27CB_0574), 1, 24, 13),
            ((0x2F69_061B, 0x4246_073A), 1, 77, 118),
        ] {
            let decoded = decode_item_operation_semantics(schema(pair), &vec![0; length], state);
            assert_eq!(decoded.effect_item_id, Some(item_id));
        }
    }

    #[test]
    fn super_magnet_state_one_has_distinct_transition_source_and_target() {
        let mut raw = vec![0; 29];
        put_u32(&mut raw, 16, 0x1111_1111);
        put_u32(&mut raw, 20, 0x2222_2222);
        put_u32(&mut raw, 24, 0x3333_3333);
        raw[28] = 1;
        let decoded = decode_item_operation_semantics(schema((0x198F_044A, 0x27F0_0569)), &raw, 1);
        assert_eq!(decoded.meaning, ItemLifecycleMeaning::Activate);
        assert_eq!(decoded.transition_token, Some(0x1111_1111));
        assert_eq!(decoded.source_object_id, Some(0x2222_2222));
        assert_eq!(decoded.target_object_id, Some(0x3333_3333));
        assert_eq!(decoded.variant, Some(1));
    }

    #[test]
    fn compact_waterbomb_transition_does_not_swap_source_and_target() {
        let mut raw = vec![0; 29];
        put_u32(&mut raw, 16, 0xAAAA_0001);
        put_u32(&mut raw, 20, 0xBBBB_0002);
        raw[24] = 7;
        put_u32(&mut raw, 25, 0xCCCC_0003);
        let decoded = decode_item_operation_semantics(schema((0x1E65_04C9, 0x2DE5_05E8)), &raw, 2);
        assert_eq!(decoded.meaning, ItemLifecycleMeaning::Impact);
        assert_eq!(decoded.native_phase, Some(2));
        assert_eq!(decoded.transition_token, Some(0xAAAA_0001));
        assert_eq!(decoded.target_object_id, Some(0xBBBB_0002));
        assert_eq!(decoded.source_object_id, Some(0xCCCC_0003));
    }

    #[test]
    fn barricade_state_two_is_trace_correlated_impact_but_state_three_is_resolve() {
        let schema = schema((0x1D86_04A3, 0x2D06_05C2));
        let raw = vec![0; 25];
        let hit = decode_item_operation_semantics(schema, &raw, 2);
        let resolve = decode_item_operation_semantics(schema, &raw, 3);
        assert_eq!(hit.meaning, ItemLifecycleMeaning::Impact);
        assert_eq!(hit.native_phase, Some(3));
        assert_eq!(hit.evidence, ItemSemanticEvidence::RetainedTraceCorrelated);
        assert_eq!(resolve.meaning, ItemLifecycleMeaning::Resolve);
        assert_eq!(resolve.native_phase, Some(4));
    }

    #[test]
    fn recovered_course_hazard_and_pirate_consumers_preserve_actor_roles() {
        let boss = schema((0x233A_0538, 0x33D9_0657));
        let mut boss_launch = vec![0; 77];
        put_u32(&mut boss_launch, 16, 0xB001);
        put_u32(&mut boss_launch, 20, 0xB002);
        put_u32(&mut boss_launch, 24, 0xB003);
        boss_launch[28] = 7;
        let decoded = decode_item_operation_semantics(boss, &boss_launch, 1);
        assert_eq!(decoded.meaning, ItemLifecycleMeaning::Launch);
        assert_eq!(decoded.native_phase, Some(0));
        assert_eq!(decoded.transition_token, Some(0xB001));
        assert_eq!(decoded.source_object_id, Some(0xB002));
        assert_eq!(decoded.target_object_id, Some(0xB003));
        assert_eq!(decoded.variant, Some(7));
        assert_eq!(decoded.evidence, ItemSemanticEvidence::ProducerAndConsumer);

        let mut boss_resolve = vec![0; 68];
        put_u32(&mut boss_resolve, 64, 0xB004);
        let decoded = decode_item_operation_semantics(boss, &boss_resolve, 3);
        assert_eq!(decoded.meaning, ItemLifecycleMeaning::Resolve);
        assert_eq!(decoded.native_phase, Some(3));
        assert_eq!(decoded.transition_token, Some(0xB004));
        let decoded = decode_item_operation_semantics(boss, &[0; 16], 4);
        assert_eq!(decoded.meaning, ItemLifecycleMeaning::Remove);
        assert_eq!(decoded.native_phase, None);

        for (pair, source_offset, launch_length, launch_meaning) in [
            (
                (0x1DB9_04A4, 0x2D39_05C3),
                53,
                63,
                ItemLifecycleMeaning::Place,
            ),
            (
                (0x14A7_03E3, 0x21E9_0502),
                85,
                91,
                ItemLifecycleMeaning::Launch,
            ),
        ] {
            let schema = schema(pair);
            let mut launch = vec![0; launch_length];
            put_u32(&mut launch, 20, 0xC001);
            put_u32(&mut launch, source_offset, 0xC002);
            launch[launch_length - 1] = 9;
            let decoded = decode_item_operation_semantics(schema, &launch, 1);
            assert_eq!(decoded.meaning, launch_meaning);
            assert_eq!(decoded.native_phase, Some(0));
            assert_eq!(decoded.transition_token, Some(0xC001));
            assert_eq!(decoded.source_object_id, Some(0xC002));
            assert_eq!(decoded.variant, Some(9));

            let mut impact = vec![0; 33];
            put_u32(&mut impact, 20, 0xC003);
            put_u32(&mut impact, 24, 0xC004);
            impact[28] = 1;
            put_u32(&mut impact, 29, 0xC005);
            let decoded = decode_item_operation_semantics(schema, &impact, 2);
            assert_eq!(decoded.meaning, ItemLifecycleMeaning::Impact);
            assert_eq!(decoded.transition_token, Some(0xC003));
            assert_eq!(decoded.target_object_id, Some(0xC004));
            assert_eq!(
                decoded.source_object_id,
                (source_offset == 85).then_some(0xC005)
            );

            impact[28] = 0;
            let decoded = decode_item_operation_semantics(schema, &impact, 3);
            assert_eq!(decoded.meaning, ItemLifecycleMeaning::Remove);
            assert_eq!(decoded.native_phase, None);
            assert_eq!(decoded.source_object_id, None);
            assert_eq!(decoded.target_object_id, None);
        }

        let course = schema((0x1139_0397, 0x0D73_0327));
        let mut goal = vec![0; 32];
        put_u32(&mut goal, 12, 0xD001);
        put_u32(&mut goal, 16, 4);
        goal[20..28].copy_from_slice(&[b'g', 0, b'o', 0, b'a', 0, b'l', 0]);
        put_u32(&mut goal, 28, 0xD002);
        let decoded = decode_item_operation_semantics(course, &goal, 0xD001);
        assert_eq!(decoded.meaning, ItemLifecycleMeaning::NoClientAction);
        assert_eq!(decoded.target_object_id, Some(0xD001));
        assert_eq!(decoded.transition_token, Some(0xD002));

        let pirate = schema((0x2369_052B, 0x3408_064A));
        let mut raw = vec![0; 28];
        put_u32(&mut raw, 16, 0xE001);
        put_u32(&mut raw, 20, 0xE002);
        put_u32(&mut raw, 24, 0xE003);
        for (state, meaning, phase) in [
            (1, ItemLifecycleMeaning::Activate, 0),
            (2, ItemLifecycleMeaning::Impact, 2),
            (3, ItemLifecycleMeaning::Remove, 3),
            (4, ItemLifecycleMeaning::Resolve, 4),
        ] {
            let decoded = decode_item_operation_semantics(pirate, &raw, state);
            assert_eq!(decoded.meaning, meaning);
            assert_eq!(decoded.native_phase, Some(phase));
            assert_eq!(decoded.transition_token, Some(0xE001));
        }
        let shielded = decode_item_operation_semantics(pirate, &raw, 4);
        assert_eq!(shielded.source_object_id, Some(0xE003));
        assert_eq!(shielded.target_object_id, Some(0xE003));
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one exhaustive table keeps all reconstructed class/state writer shapes auditable together"
    )]
    fn every_implemented_external_state_maps_after_its_exact_writer_shape() {
        #[derive(Clone, Copy)]
        struct Case {
            pair: (u32, u32),
            state: u32,
            length: usize,
            meaning: ItemLifecycleMeaning,
            phase: Option<u8>,
        }

        const BARRICADE: (u32, u32) = (0x1D86_04A3, 0x2D06_05C2);
        const BANANA: (u32, u32) = (0x1090_0367, 0x1CB3_0486);
        const MINE: (u32, u32) = (0x0A6B_02AF, 0x1450_03CE);
        const ROCKET: (u32, u32) = (0x1129_038E, 0x1D4C_04AD);
        const SUPER_MAG: (u32, u32) = (0x198F_044A, 0x27F0_0569);
        const WATERBOMB: (u32, u32) = (0x1E65_04C9, 0x2DE5_05E8);
        const WATERFLY: (u32, u32) = (0x19AE_0474, 0x280F_0593);
        let cases = [
            Case {
                pair: BARRICADE,
                state: 0,
                length: 25,
                meaning: ItemLifecycleMeaning::Initialize,
                phase: Some(0),
            },
            Case {
                pair: BARRICADE,
                state: 1,
                length: 73,
                meaning: ItemLifecycleMeaning::Place,
                phase: Some(1),
            },
            Case {
                pair: BARRICADE,
                state: 2,
                length: 25,
                meaning: ItemLifecycleMeaning::Impact,
                phase: Some(3),
            },
            Case {
                pair: BARRICADE,
                state: 3,
                length: 25,
                meaning: ItemLifecycleMeaning::Resolve,
                phase: Some(4),
            },
            Case {
                pair: BARRICADE,
                state: 4,
                length: 26,
                meaning: ItemLifecycleMeaning::Remove,
                phase: None,
            },
            Case {
                pair: BANANA,
                state: 1,
                length: 74,
                meaning: ItemLifecycleMeaning::Place,
                phase: Some(0),
            },
            Case {
                pair: BANANA,
                state: 2,
                length: 30,
                meaning: ItemLifecycleMeaning::Impact,
                phase: None,
            },
            Case {
                pair: BANANA,
                state: 3,
                length: 30,
                meaning: ItemLifecycleMeaning::Resolve,
                phase: None,
            },
            Case {
                pair: MINE,
                state: 1,
                length: 77,
                meaning: ItemLifecycleMeaning::Place,
                phase: Some(0),
            },
            Case {
                pair: MINE,
                state: 2,
                length: 29,
                meaning: ItemLifecycleMeaning::Impact,
                phase: None,
            },
            Case {
                pair: MINE,
                state: 3,
                length: 29,
                meaning: ItemLifecycleMeaning::NoClientAction,
                phase: None,
            },
            Case {
                pair: MINE,
                state: 4,
                length: 29,
                meaning: ItemLifecycleMeaning::Resolve,
                phase: Some(4),
            },
            Case {
                pair: MINE,
                state: 5,
                length: 29,
                meaning: ItemLifecycleMeaning::Remove,
                phase: None,
            },
            Case {
                pair: MINE,
                state: 6,
                length: 68,
                meaning: ItemLifecycleMeaning::Respawn,
                phase: Some(0),
            },
            Case {
                pair: ROCKET,
                state: 1,
                length: 82,
                meaning: ItemLifecycleMeaning::Launch,
                phase: Some(1),
            },
            Case {
                pair: ROCKET,
                state: 2,
                length: 73,
                meaning: ItemLifecycleMeaning::Impact,
                phase: Some(2),
            },
            Case {
                pair: ROCKET,
                state: 3,
                length: 20,
                meaning: ItemLifecycleMeaning::Resolve,
                phase: Some(3),
            },
            Case {
                pair: ROCKET,
                state: 4,
                length: 20,
                meaning: ItemLifecycleMeaning::Retarget,
                phase: None,
            },
            Case {
                pair: ROCKET,
                state: 5,
                length: 16,
                meaning: ItemLifecycleMeaning::Remove,
                phase: None,
            },
            Case {
                pair: ROCKET,
                state: 6,
                length: 16,
                meaning: ItemLifecycleMeaning::NoClientAction,
                phase: None,
            },
            Case {
                pair: ROCKET,
                state: 7,
                length: 24,
                meaning: ItemLifecycleMeaning::RebindSource,
                phase: Some(6),
            },
            Case {
                pair: ROCKET,
                state: 8,
                length: 20,
                meaning: ItemLifecycleMeaning::Resolve,
                phase: Some(7),
            },
            Case {
                pair: ROCKET,
                state: 9,
                length: 20,
                meaning: ItemLifecycleMeaning::Resolve,
                phase: Some(8),
            },
            Case {
                pair: ROCKET,
                state: 10,
                length: 20,
                meaning: ItemLifecycleMeaning::Resolve,
                phase: Some(9),
            },
            Case {
                pair: SUPER_MAG,
                state: 1,
                length: 29,
                meaning: ItemLifecycleMeaning::Activate,
                phase: Some(0),
            },
            Case {
                pair: SUPER_MAG,
                state: 2,
                length: 16,
                meaning: ItemLifecycleMeaning::Resolve,
                phase: Some(4),
            },
            Case {
                pair: WATERBOMB,
                state: 1,
                length: 125,
                meaning: ItemLifecycleMeaning::Launch,
                phase: Some(0),
            },
            Case {
                pair: WATERBOMB,
                state: 2,
                length: 29,
                meaning: ItemLifecycleMeaning::Impact,
                phase: Some(2),
            },
            Case {
                pair: WATERBOMB,
                state: 3,
                length: 29,
                meaning: ItemLifecycleMeaning::Impact,
                phase: Some(3),
            },
            Case {
                pair: WATERBOMB,
                state: 4,
                length: 29,
                meaning: ItemLifecycleMeaning::Remove,
                phase: Some(4),
            },
            Case {
                pair: WATERFLY,
                state: 1,
                length: 77,
                meaning: ItemLifecycleMeaning::Launch,
                phase: Some(0),
            },
            Case {
                pair: WATERFLY,
                state: 2,
                length: 65,
                meaning: ItemLifecycleMeaning::Impact,
                phase: None,
            },
            Case {
                pair: WATERFLY,
                state: 3,
                length: 68,
                meaning: ItemLifecycleMeaning::Impact,
                phase: Some(2),
            },
            Case {
                pair: WATERFLY,
                state: 4,
                length: 28,
                meaning: ItemLifecycleMeaning::Resolve,
                phase: Some(3),
            },
            Case {
                pair: WATERFLY,
                state: 5,
                length: 20,
                meaning: ItemLifecycleMeaning::Remove,
                phase: Some(4),
            },
        ];

        for case in cases {
            let schema = schema(case.pair);
            let mut raw = vec![0; case.length];
            put_u32(&mut raw, 8, 0x7000_0001);
            match schema.state_field {
                crate::game_slot_item_schema::ItemOperationStateField::U8 { offset } => {
                    raw[offset] = u8::try_from(case.state).unwrap();
                }
                crate::game_slot_item_schema::ItemOperationStateField::U32 { offset } => {
                    put_u32(&mut raw, offset, case.state);
                }
            }
            let validated = schema.validate(&raw).unwrap();
            let decoded = decode_item_operation_semantics(schema, &raw, validated.state());
            assert_eq!(
                decoded.meaning, case.meaning,
                "{} state {}",
                schema.class_name, case.state
            );
            assert_eq!(
                decoded.native_phase, case.phase,
                "{} state {}",
                schema.class_name, case.state
            );
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one family table keeps every recovered writer state adjacent and auditable"
    )]
    fn second_semantic_pass_maps_bomb_and_mine_families() {
        let ordinary_bomb_states = [
            (1_u32, 120_usize, ItemLifecycleMeaning::Launch, Some(0)),
            (2, 28, ItemLifecycleMeaning::Impact, Some(2)),
            (3, 28, ItemLifecycleMeaning::Resolve, Some(3)),
            (4, 28, ItemLifecycleMeaning::Resolve, Some(4)),
        ];
        let cases = [
            ((0x1900_0448, 0x2761_0567), ordinary_bomb_states),
            ((0x19EB_046D, 0x284C_058C), ordinary_bomb_states),
        ];
        for (pair, states) in cases {
            let schema = schema(pair);
            for (state, length, meaning, phase) in states {
                let mut raw = vec![0; length];
                put_u32(&mut raw, 8, 0x7400_0001);
                put_u32(&mut raw, 12, state);
                let validated = schema.validate(&raw).unwrap();
                let decoded = decode_item_operation_semantics(schema, &raw, validated.state());
                assert_eq!(
                    (decoded.meaning, decoded.native_phase),
                    (meaning, phase),
                    "{} state {state}",
                    schema.class_name
                );
            }
        }

        let cases = [
            (
                (0x2DC1_05C8, 0x409E_06E7),
                vec![
                    (1_u32, 121_usize, ItemLifecycleMeaning::Launch, Some(0)),
                    (2, 33, ItemLifecycleMeaning::Impact, Some(2)),
                    (3, 33, ItemLifecycleMeaning::Resolve, Some(4)),
                ],
            ),
            (
                (0x42E4_071F, 0x591E_083E),
                vec![
                    (1, 132, ItemLifecycleMeaning::Launch, Some(0)),
                    (2, 28, ItemLifecycleMeaning::Impact, Some(2)),
                    (3, 24, ItemLifecycleMeaning::Resolve, Some(3)),
                    (4, 24, ItemLifecycleMeaning::Resolve, Some(4)),
                ],
            ),
            (
                (0x2954_059D, 0x3B12_06BC),
                vec![
                    (1, 132, ItemLifecycleMeaning::Launch, Some(0)),
                    (2, 28, ItemLifecycleMeaning::Impact, Some(2)),
                    (3, 24, ItemLifecycleMeaning::Resolve, Some(3)),
                    (4, 24, ItemLifecycleMeaning::Resolve, Some(4)),
                ],
            ),
            (
                (0x6381_08BF, 0x7E37_09DE),
                vec![
                    (1, 132, ItemLifecycleMeaning::Launch, Some(0)),
                    (2, 32, ItemLifecycleMeaning::Impact, Some(2)),
                    (3, 28, ItemLifecycleMeaning::Resolve, Some(4)),
                ],
            ),
            (
                (0x41CC_070F, 0x3996_067F),
                vec![
                    (1, 24, ItemLifecycleMeaning::Launch, Some(0)),
                    (2, 28, ItemLifecycleMeaning::Impact, Some(2)),
                    (3, 24, ItemLifecycleMeaning::Resolve, Some(3)),
                    (4, 24, ItemLifecycleMeaning::Resolve, Some(4)),
                ],
            ),
            (
                (0x2DDA_05D7, 0x40B7_06F6),
                vec![
                    (1, 24, ItemLifecycleMeaning::Launch, Some(0)),
                    (2, 28, ItemLifecycleMeaning::Impact, Some(2)),
                    (3, 24, ItemLifecycleMeaning::Resolve, Some(3)),
                    (4, 24, ItemLifecycleMeaning::Resolve, Some(4)),
                ],
            ),
            (
                (0x48D7_0757, 0x6030_0876),
                vec![
                    (1, 24, ItemLifecycleMeaning::Launch, Some(0)),
                    (2, 32, ItemLifecycleMeaning::Impact, Some(2)),
                    (3, 28, ItemLifecycleMeaning::Resolve, Some(4)),
                ],
            ),
            (
                (0x2EC5_05FC, 0x41A2_071B),
                vec![
                    (1, 24, ItemLifecycleMeaning::Launch, Some(0)),
                    (2, 28, ItemLifecycleMeaning::Impact, Some(2)),
                    (3, 28, ItemLifecycleMeaning::Resolve, Some(3)),
                    (4, 28, ItemLifecycleMeaning::Resolve, Some(4)),
                ],
            ),
            (
                (0x196A_0455, 0x27CB_0574),
                vec![
                    (1, 24, ItemLifecycleMeaning::Launch, Some(0)),
                    (2, 28, ItemLifecycleMeaning::Impact, Some(2)),
                    (3, 28, ItemLifecycleMeaning::Resolve, Some(3)),
                    (4, 28, ItemLifecycleMeaning::Resolve, Some(4)),
                ],
            ),
            (
                (0x1E04_04B2, 0x2D84_05D1),
                vec![
                    (1, 73, ItemLifecycleMeaning::Place, Some(0)),
                    (2, 29, ItemLifecycleMeaning::Impact, Some(2)),
                    (3, 29, ItemLifecycleMeaning::Resolve, Some(3)),
                    (4, 29, ItemLifecycleMeaning::Resolve, Some(4)),
                    (7, 29, ItemLifecycleMeaning::NoClientAction, None),
                ],
            ),
            (
                (0x1909_043E, 0x276A_055D),
                vec![
                    (1, 85, ItemLifecycleMeaning::Place, Some(0)),
                    (2, 33, ItemLifecycleMeaning::Impact, None),
                    (3, 33, ItemLifecycleMeaning::Resolve, None),
                    (4, 33, ItemLifecycleMeaning::NoClientAction, None),
                    (5, 24, ItemLifecycleMeaning::Resolve, None),
                ],
            ),
        ];
        for (pair, states) in cases {
            let schema = schema(pair);
            for (state, length, meaning, phase) in states {
                let mut raw = vec![0; length];
                put_u32(&mut raw, 8, 0x7400_0002);
                match schema.state_field {
                    crate::game_slot_item_schema::ItemOperationStateField::U8 { offset } => {
                        raw[offset] = u8::try_from(state).unwrap();
                    }
                    crate::game_slot_item_schema::ItemOperationStateField::U32 { offset } => {
                        put_u32(&mut raw, offset, state);
                    }
                }
                if schema.class_name == "GopTimeMine" && matches!(state, 2 | 3) {
                    raw[28] = 1;
                }
                let validated = schema.validate(&raw).unwrap();
                let decoded = decode_item_operation_semantics(schema, &raw, validated.state());
                assert_eq!(
                    (decoded.meaning, decoded.native_phase),
                    (meaning, phase),
                    "{} state {state}",
                    schema.class_name
                );
            }
        }
    }

    #[test]
    fn time_mine_target_branch_is_flagged_and_runtime_conditional() {
        let schema = schema((0x1909_043E, 0x276A_055D));
        let mut raw = vec![0; 33];
        put_u32(&mut raw, 8, 0x7400_0010);
        put_u32(&mut raw, 16, 2);
        put_u32(&mut raw, 20, 0x7500_0010);
        put_u32(&mut raw, 24, 0x7600_0010);
        put_u32(&mut raw, 29, 0x7700_0010);

        let no_target = decode_item_operation_semantics(schema, &raw, 2);
        assert_eq!(no_target.meaning, ItemLifecycleMeaning::Resolve);
        assert_eq!(no_target.native_phase, None);
        assert_eq!(no_target.source_object_id, None);
        assert_eq!(no_target.target_object_id, None);

        raw[28] = 1;
        let conditional_impact = decode_item_operation_semantics(schema, &raw, 2);
        assert_eq!(conditional_impact.meaning, ItemLifecycleMeaning::Impact);
        assert_eq!(conditional_impact.native_phase, None);
        assert_eq!(conditional_impact.source_object_id, None);
        assert_eq!(conditional_impact.target_object_id, Some(0x7600_0010));
    }

    #[test]
    fn recovered_runtime_consumers_keep_their_exact_actor_bindings() {
        const TOKEN: u32 = 0x7100_0001;
        const TARGET: u32 = 0x7200_0002;
        const SOURCE: u32 = 0x7300_0003;

        let big = schema((0x276B_0567, 0x3929_0686));
        let mut raw = vec![0; 29];
        put_u32(&mut raw, 8, 0x7000_0000);
        raw[12] = 0xA5;
        put_u32(&mut raw, 13, 4);
        put_u32(&mut raw, 17, TOKEN);
        put_u32(&mut raw, 21, TARGET);
        put_u32(&mut raw, 25, SOURCE);
        let validated = big.validate(&raw).unwrap();
        assert_eq!(validated.state(), 4);
        let decoded = decode_item_operation_semantics(big, &raw, validated.state());
        assert_eq!(decoded.meaning, ItemLifecycleMeaning::Resolve);
        assert_eq!(decoded.native_phase, Some(4));
        assert_eq!(decoded.transition_token, Some(TOKEN));
        assert_eq!(decoded.target_object_id, Some(TARGET));
        assert_eq!(decoded.source_object_id, Some(SOURCE));
        assert_eq!(decoded.variant, Some(0xA5));

        for (phase, meaning) in [
            (0, ItemLifecycleMeaning::Activate),
            (2, ItemLifecycleMeaning::Impact),
            (3, ItemLifecycleMeaning::Impact),
            (4, ItemLifecycleMeaning::Resolve),
        ] {
            put_u32(&mut raw, 13, phase);
            let decoded = decode_item_operation_semantics(big, &raw, phase);
            assert_eq!(decoded.meaning, meaning);
            assert_eq!(decoded.native_phase, u8::try_from(phase).ok());
        }

        for pair in [
            (0x41CC_070F, 0x3996_067F),
            (0x2DDA_05D7, 0x40B7_06F6),
            (0x2954_059D, 0x3B12_06BC),
        ] {
            let schema = schema(pair);
            let mut raw = vec![0; 28];
            put_u32(&mut raw, 12, 2);
            put_u32(&mut raw, 16, TOKEN);
            put_u32(&mut raw, 20, TARGET);
            put_u32(&mut raw, 24, SOURCE);
            let decoded = decode_item_operation_semantics(schema, &raw, 2);
            assert_eq!(decoded.meaning, ItemLifecycleMeaning::Impact);
            assert_eq!(decoded.target_object_id, Some(TARGET));
            assert_eq!(decoded.source_object_id, Some(SOURCE));
        }

        let infected = schema((0x48D7_0757, 0x6030_0876));
        let mut raw = vec![0; 32];
        put_u32(&mut raw, 12, 2);
        put_u32(&mut raw, 16, TOKEN);
        put_u32(&mut raw, 20, TARGET);
        put_u32(&mut raw, 24, 0x1122_3344);
        put_u32(&mut raw, 28, SOURCE);
        let decoded = decode_item_operation_semantics(infected, &raw, 2);
        assert_eq!(decoded.target_object_id, Some(TARGET));
        assert_eq!(decoded.source_object_id, Some(SOURCE));

        let snow = schema((0x2EC5_05FC, 0x41A2_071B));
        let mut raw = vec![0; 28];
        put_u32(&mut raw, 12, 3);
        put_u32(&mut raw, 16, TOKEN);
        put_u32(&mut raw, 20, TARGET);
        put_u32(&mut raw, 24, SOURCE);
        let decoded = decode_item_operation_semantics(snow, &raw, 3);
        assert_eq!(decoded.target_object_id, Some(TARGET));
        assert_eq!(decoded.source_object_id, Some(TARGET));
    }

    #[test]
    fn each_rocket_variant_keeps_its_own_consumer_followup_limit() {
        let variants = [
            ((0x2261_0510, 0x3300_062F), 10_u32),
            ((0x3A06_069F, 0x4F21_07BE), 9),
            ((0x228A_0514, 0x3329_0633), 10),
            ((0x1584_0409, 0x22C6_0528), 7),
            ((0x2882_0589, 0x3A40_06A8), 9),
        ];
        for (pair, last_transition_state) in variants {
            let schema = schema(pair);
            for state in 1..=10 {
                let length = match state {
                    1 => 77,
                    2 => 72,
                    3 if schema.class_name == "GopGoldRocket" => 22,
                    3 | 4 | 8..=10 => 20,
                    5 | 6 => 16,
                    7 => 24,
                    _ => unreachable!(),
                };
                let mut raw = vec![0; length];
                put_u32(&mut raw, 8, 0x7000_0002);
                put_u32(&mut raw, 12, state);
                let validated = schema.validate(&raw).unwrap();
                let decoded = decode_item_operation_semantics(schema, &raw, validated.state());
                let expected = match state {
                    1 => (ItemLifecycleMeaning::Launch, Some(1)),
                    2 => (ItemLifecycleMeaning::Impact, Some(2)),
                    3 => (ItemLifecycleMeaning::Resolve, Some(3)),
                    4 => (ItemLifecycleMeaning::Retarget, None),
                    5 => (ItemLifecycleMeaning::Remove, None),
                    6 => (ItemLifecycleMeaning::NoClientAction, None),
                    7 => (ItemLifecycleMeaning::RebindSource, Some(6)),
                    value if value <= last_transition_state => {
                        (ItemLifecycleMeaning::Resolve, u8::try_from(value - 1).ok())
                    }
                    _ => (ItemLifecycleMeaning::NoClientAction, None),
                };
                assert_eq!(
                    (decoded.meaning, decoded.native_phase),
                    expected,
                    "{} state {state}",
                    schema.class_name
                );
            }
        }
    }

    #[test]
    fn waterfly_variants_keep_their_individual_terminal_contracts() {
        let shared_variants = [(0x49AB_0796, 0x6104_08B5), (0x2F69_061B, 0x4246_073A)];
        let shared_states = [
            (1_u32, 77_usize, ItemLifecycleMeaning::Launch, Some(0)),
            (2, 65, ItemLifecycleMeaning::Impact, None),
            (3, 68, ItemLifecycleMeaning::Impact, Some(2)),
            (4, 28, ItemLifecycleMeaning::Resolve, Some(3)),
            (5, 20, ItemLifecycleMeaning::Remove, Some(4)),
        ];

        for pair in shared_variants {
            let schema = schema(pair);
            for (state, length, meaning, phase) in shared_states {
                let mut raw = vec![0; length];
                put_u32(&mut raw, 8, 0x7000_0003);
                put_u32(&mut raw, 12, state);
                let validated = schema.validate(&raw).unwrap();
                let decoded = decode_item_operation_semantics(schema, &raw, validated.state());
                assert_eq!(
                    (decoded.meaning, decoded.native_phase),
                    (meaning, phase),
                    "{} state {state}",
                    schema.class_name
                );
            }
        }

        let schema = schema((0x2EE3_05F4, 0x41C0_0713));
        let waterbomb_fly_states = [
            (1_u32, 77_usize, ItemLifecycleMeaning::Launch, Some(0)),
            (2, 64, ItemLifecycleMeaning::Impact, None),
            (3, 28, ItemLifecycleMeaning::Resolve, Some(3)),
            (4, 28, ItemLifecycleMeaning::Resolve, Some(4)),
            (5, 20, ItemLifecycleMeaning::NoClientAction, None),
            (6, 16, ItemLifecycleMeaning::Remove, None),
        ];
        for (state, length, meaning, phase) in waterbomb_fly_states {
            let mut raw = vec![0; length];
            put_u32(&mut raw, 8, 0x7000_0004);
            put_u32(&mut raw, 12, state);
            let validated = schema.validate(&raw).unwrap();
            let decoded = decode_item_operation_semantics(schema, &raw, validated.state());
            assert_eq!(
                (decoded.meaning, decoded.native_phase),
                (meaning, phase),
                "{} state {state}",
                schema.class_name
            );
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the literal table keeps each recovered and anomalous branch auditable"
    )]
    fn fourth_pass_keeps_exact_shapes_separate_from_proven_meanings() {
        const TOKEN: u32 = 0x7100_0001;
        const SOURCE: u32 = 0x7200_0002;
        const TARGET: u32 = 0x7300_0003;

        struct Case {
            pair: (u32, u32),
            state: u32,
            length: usize,
            token_offset: Option<usize>,
            source_offset: Option<usize>,
            target_offset: Option<usize>,
            variant_offset: Option<usize>,
            meaning: ItemLifecycleMeaning,
            phase: Option<u8>,
        }

        let cases = [
            Case {
                pair: (0x0D49_030D, 0x184D_042C),
                state: 0,
                length: 25,
                token_offset: Some(16),
                source_offset: Some(21),
                target_offset: None,
                variant_offset: Some(20),
                meaning: ItemLifecycleMeaning::Activate,
                phase: Some(0),
            },
            Case {
                pair: (0x0D49_030D, 0x184D_042C),
                state: 2,
                length: 28,
                token_offset: Some(16),
                source_offset: Some(20),
                target_offset: Some(24),
                variant_offset: None,
                meaning: ItemLifecycleMeaning::Impact,
                phase: Some(2),
            },
            Case {
                pair: (0x07AE_0248, 0x1074_0367),
                state: 0,
                length: 26,
                token_offset: Some(16),
                source_offset: Some(22),
                target_offset: None,
                variant_offset: Some(20),
                meaning: ItemLifecycleMeaning::Activate,
                phase: Some(0),
            },
            Case {
                pair: (0x0D8B_032B, 0x188F_044A),
                state: 1,
                length: 29,
                token_offset: Some(16),
                source_offset: Some(20),
                target_offset: Some(24),
                variant_offset: Some(28),
                meaning: ItemLifecycleMeaning::Activate,
                phase: Some(0),
            },
            Case {
                pair: (0x10C3_0382, 0x1CE6_04A1),
                state: 1,
                length: 78,
                token_offset: Some(16),
                source_offset: Some(20),
                target_offset: Some(24),
                variant_offset: Some(77),
                meaning: ItemLifecycleMeaning::Launch,
                phase: Some(0),
            },
            Case {
                pair: (0x1942_0457, 0x27A3_0576),
                state: 1,
                length: 30,
                token_offset: Some(20),
                source_offset: Some(16),
                target_offset: Some(24),
                variant_offset: Some(28),
                meaning: ItemLifecycleMeaning::Activate,
                phase: Some(0),
            },
            Case {
                pair: (0x196B_0451, 0x27CC_0570),
                state: 1,
                length: 29,
                token_offset: Some(24),
                source_offset: Some(16),
                target_offset: None,
                variant_offset: Some(28),
                meaning: ItemLifecycleMeaning::Activate,
                phase: Some(0),
            },
            Case {
                pair: (0x196B_0451, 0x27CC_0570),
                state: 2,
                length: 29,
                token_offset: Some(24),
                source_offset: Some(16),
                target_offset: Some(20),
                variant_offset: Some(28),
                meaning: ItemLifecycleMeaning::Impact,
                phase: Some(1),
            },
            Case {
                pair: (0x2E54_05E8, 0x4131_0707),
                state: 0,
                length: 26,
                token_offset: Some(16),
                source_offset: Some(21),
                target_offset: None,
                variant_offset: Some(20),
                meaning: ItemLifecycleMeaning::Activate,
                phase: None,
            },
            Case {
                pair: (0x2262_0502, 0x3301_0621),
                state: 0,
                length: 30,
                token_offset: Some(16),
                source_offset: Some(24),
                target_offset: Some(20),
                variant_offset: Some(29),
                meaning: ItemLifecycleMeaning::Launch,
                phase: Some(0),
            },
            Case {
                pair: (0x2262_0502, 0x3301_0621),
                state: 4,
                length: 29,
                token_offset: Some(16),
                source_offset: None,
                target_offset: Some(20),
                variant_offset: Some(28),
                meaning: ItemLifecycleMeaning::Resolve,
                phase: Some(6),
            },
            Case {
                pair: (0x3C6F_06D4, 0x518A_07F3),
                state: 1,
                length: 58,
                token_offset: Some(16),
                source_offset: Some(20),
                target_offset: None,
                variant_offset: Some(56),
                meaning: ItemLifecycleMeaning::Launch,
                phase: Some(1),
            },
            Case {
                pair: (0x3C6F_06D4, 0x518A_07F3),
                state: 2,
                length: 24,
                token_offset: None,
                source_offset: None,
                target_offset: None,
                variant_offset: None,
                meaning: ItemLifecycleMeaning::NoClientAction,
                phase: None,
            },
            Case {
                pair: (0x3C6F_06D4, 0x518A_07F3),
                state: 3,
                length: 24,
                token_offset: None,
                source_offset: None,
                target_offset: None,
                variant_offset: None,
                meaning: ItemLifecycleMeaning::NoClientAction,
                phase: None,
            },
        ];

        for case in cases {
            let schema = schema(case.pair);
            let mut raw = vec![0_u8; case.length];
            put_u32(&mut raw, 8, 0x7000_0001);
            put_u32(&mut raw, 12, case.state);
            if let Some(offset) = case.token_offset {
                put_u32(&mut raw, offset, TOKEN);
            }
            if let Some(offset) = case.source_offset {
                put_u32(&mut raw, offset, SOURCE);
            }
            if let Some(offset) = case.target_offset {
                put_u32(&mut raw, offset, TARGET);
            }
            if let Some(offset) = case.variant_offset {
                raw[offset] = 0x5A;
            }

            let validated = schema.validate(&raw).unwrap();
            let decoded = decode_item_operation_semantics(schema, &raw, validated.state());
            assert_eq!(decoded.meaning, case.meaning, "{}", schema.class_name);
            assert_eq!(decoded.native_phase, case.phase, "{}", schema.class_name);
            assert_eq!(
                decoded.transition_token,
                case.token_offset.map(|_| TOKEN),
                "{}",
                schema.class_name
            );
            assert_eq!(
                decoded.source_object_id,
                case.source_offset.map(|_| SOURCE),
                "{}",
                schema.class_name
            );
            assert_eq!(
                decoded.target_object_id,
                case.target_offset.map(|_| TARGET),
                "{}",
                schema.class_name
            );
            assert_eq!(
                decoded.variant,
                case.variant_offset.map(|_| 0x5A),
                "{}",
                schema.class_name
            );
        }

        let spacecraft = schema((0x2262_0502, 0x3301_0621));
        let mut consumer_only = vec![0_u8; 29];
        put_u32(&mut consumer_only, 12, 3);
        assert_eq!(
            decode_item_operation_semantics(spacecraft, &consumer_only, 3).evidence,
            ItemSemanticEvidence::StaticConsumer
        );
        let mut produced_runtime_flag = vec![0_u8; 17];
        put_u32(&mut produced_runtime_flag, 12, 7);
        produced_runtime_flag[16] = 1;
        assert_eq!(
            decode_item_operation_semantics(spacecraft, &produced_runtime_flag, 7).evidence,
            ItemSemanticEvidence::ProducerAndConsumer
        );
    }
}
