use super::*;

/// Ordered plane/tile/XP milestones from selected gnome_course.rs2 dests.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GnomeCourseCycle {
    pub log: Option<Observation>,
    pub ground_return: Option<Observation>,
    pub pipe: Option<Observation>,
    pub second_lap: bool,
}

impl GnomeCourseCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        let xp = now.skill_xp("agility");
        if self.log.is_none()
            && xp > baseline.skill_xp("agility")
            && near(now.tile, GNOME_AFTER_LOG, 3)
        {
            self.log = Some(now.clone());
        }
        if let Some(log) = &self.log {
            if self.ground_return.is_none()
                && xp > log.skill_xp("agility")
                && near(now.tile, GNOME_GROUND_RETURN, 3)
            {
                self.ground_return = Some(now.clone());
            }
        }
        if let Some(ground) = &self.ground_return {
            if self.pipe.is_none()
                && xp > ground.skill_xp("agility")
                && near(now.tile, GNOME_PIPE, 6)
            {
                self.pipe = Some(now.clone());
            }
        }
        if let Some(pipe) = &self.pipe {
            self.second_lap |= xp > pipe.skill_xp("agility")
                && xp - baseline.skill_xp("agility") >= GNOME_SECOND_LOG_XP
                && near(now.tile, GNOME_AFTER_LOG, 3);
        }
    }

    pub fn qualified(&self) -> bool {
        self.second_lap
    }
}

/// Ordered ridge, five-obstacle lap bonus, and next-pipe milestones from the
/// selected m46_61 loc destinations. A queued click or aggregate XP alone
/// cannot advance the chain.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WildyAgilityCycle {
    pub ridge: Option<Observation>,
    pub pipe: Option<Observation>,
    pub rope: Option<Observation>,
    pub stone: Option<Observation>,
    pub log: Option<Observation>,
    pub rocks: Option<Observation>,
    pub further_pipe: bool,
}

impl WildyAgilityCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        let xp = now.skill_xp("agility");
        let baseline_sequence = baseline
            .chat
            .iter()
            .map(|(sequence, _)| *sequence)
            .max()
            .unwrap_or(i32::MIN);
        let fresh_chat = |needle: &str| {
            now.chat.iter().any(|(sequence, text)| {
                *sequence > baseline_sequence
                    && text
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase())
            })
        };
        let on_course = now.tile.is_some_and(|(x, z, level)| {
            level == 0 && (3932..=3967).contains(&z) && (x - 2998).abs() <= 24
        });
        let ridge_failed = fresh_chat("lose your footing and fall into the wolf pit");
        if self.ridge.is_none()
            && on_course
            && !ridge_failed
            && (xp - baseline.skill_xp("agility") >= 15
                || fresh_chat("skillfully balance across the ridge"))
        {
            self.ridge = Some(now.clone());
        }
        if let Some(ridge) = &self.ridge {
            if self.pipe.is_none()
                && xp > ridge.skill_xp("agility")
                && near(now.tile, WILDY_PIPE_DEST, 3)
            {
                self.pipe = Some(now.clone());
            }
        }
        if let Some(pipe) = &self.pipe {
            if self.rope.is_none()
                && xp > pipe.skill_xp("agility")
                && near(now.tile, WILDY_ROPE_DEST, 3)
            {
                self.rope = Some(now.clone());
            }
        }
        if let Some(rope) = &self.rope {
            if self.stone.is_none()
                && xp > rope.skill_xp("agility")
                && near(now.tile, WILDY_STONE_DEST, 3)
            {
                self.stone = Some(now.clone());
            }
        }
        if let Some(stone) = &self.stone {
            if self.log.is_none()
                && xp > stone.skill_xp("agility")
                && near(now.tile, WILDY_LOG_DEST, 3)
            {
                self.log = Some(now.clone());
            }
        }
        if let Some(log) = &self.log {
            if self.rocks.is_none()
                && xp > log.skill_xp("agility")
                && xp - baseline.skill_xp("agility") >= WILDY_LAP_XP
                && near(now.tile, WILDY_ROCKS_DEST, 3)
            {
                self.rocks = Some(now.clone());
            }
        }
        if let Some(rocks) = &self.rocks {
            self.further_pipe |= xp > rocks.skill_xp("agility")
                && xp - baseline.skill_xp("agility") >= WILDY_FURTHER_XP
                && near(now.tile, WILDY_PIPE_DEST, 3);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further_pipe
    }
}

pub fn brimhaven_platform(tile: Option<(i32, i32, i32)>) -> Option<usize> {
    let (x, z, level) = tile?;
    if level != 3 {
        return None;
    }
    let xs = [2761, 2772, 2783, 2794, 2805];
    let zs = [9546, 9557, 9568, 9579, 9590];
    for (row, center_z) in zs.into_iter().enumerate() {
        for (column, center_x) in xs.into_iter().enumerate() {
            if (x - center_x).abs() <= 4 && (z - center_z).abs() <= 4 {
                return Some(row * xs.len() + column);
            }
        }
    }
    None
}

/// Ordered natural entrance fee, arena movement, first tag, first ticket, and
/// work after the ticket. Platform indexing only recognizes the selected
/// 5-by-5 arena centers and does not reproduce the foreign route planner.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BrimhavenAgilityCycle {
    pub paid: bool,
    pub entered: Option<(usize, Observation)>,
    pub moved: Option<(usize, Observation)>,
    pub first_tag: bool,
    pub ticket: Option<(usize, Observation)>,
    pub subsequent_work: bool,
}

impl BrimhavenAgilityCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        let varp = now.varp(BRIMHAVEN_ARENA_VARP);
        self.paid |= baseline.item_id(COINS_ID) - now.item_id(COINS_ID) >= 200 && varp & 0b10 != 0;
        if !self.paid {
            return;
        }

        let platform = brimhaven_platform(now.tile);
        if self.entered.is_none() {
            if let Some(platform) = platform {
                self.entered = Some((platform, now.clone()));
            }
            return;
        }
        if self.moved.is_none() {
            let (entered_platform, entered) = self.entered.as_ref().unwrap();
            if platform.is_some_and(|platform| platform != *entered_platform)
                && now.skill_xp("agility") > entered.skill_xp("agility")
            {
                self.moved = Some((platform.unwrap(), now.clone()));
            }
            return;
        }
        if !self.first_tag {
            let baseline_sequence = baseline
                .chat
                .iter()
                .map(|(sequence, _)| *sequence)
                .max()
                .unwrap_or(i32::MIN);
            let fresh_next_pillar = now.chat.iter().any(|(sequence, text)| {
                *sequence > baseline_sequence && text.to_ascii_lowercase().contains("tag the next")
            });
            if varp & 0b1111 == 0b1111 && now.item_id(BRIMHAVEN_TICKET_ID) == 0 && fresh_next_pillar
            {
                self.first_tag = true;
            }
            return;
        }
        if self.ticket.is_none() {
            if now.item_id(BRIMHAVEN_TICKET_ID) >= 1 {
                if let Some(platform) = platform {
                    self.ticket = Some((platform, now.clone()));
                }
            }
            return;
        }
        let (ticket_platform, ticket) = self.ticket.as_ref().unwrap();
        self.subsequent_work |= platform.is_some_and(|platform| platform != *ticket_platform)
            || now.skill_xp("agility") > ticket.skill_xp("agility");
    }

    pub fn qualified(&self) -> bool {
        self.subsequent_work
    }
}
