// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! What changed on the output since the last render.

use std::collections::HashMap;

use bmc::compositor::InstanceId;
use smithay::{
    utils::{Buffer, Rectangle},
    wayland::compositor::Damage,
};

/// Commits a widget may owe a render before its damage counts as its whole buffer.
///
/// A render clears what is owed, and a healthy widget commits once or twice in between.
/// More means renders have stalled, on a failing render or a flip that never completes,
/// and the regions would pile up without bound for the frame that finally draws them.
const MAX_OWED_COMMITS: usize = 8;

/// The change since the last render.
#[derive(Debug)]
pub enum OutputDamage {
    Full,
    /// The widgets that committed, with what each owes the next render.
    Widgets(HashMap<InstanceId, Owed>),
}

impl Default for OutputDamage {
    fn default() -> Self {
        Self::Widgets(HashMap::new())
    }
}

/// What a widget owes the next render, and how many commits it came from.
#[derive(Debug)]
pub struct Owed {
    damage: WidgetDamage,
    commits: usize,
}

/// What changed in a widget's buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WidgetDamage {
    Whole,
    Regions(Vec<Rectangle<i32, Buffer>>),
}

impl OutputDamage {
    pub fn mark_full(&mut self) {
        *self = Self::Full;
    }

    /// Adds a commit's damage to what the widget already owes the next render.
    pub fn mark_widget(&mut self, instance_id: &InstanceId, damage: WidgetDamage) {
        let Self::Widgets(widgets) = self else {
            return;
        };
        let Some(owed) = widgets.get_mut(instance_id) else {
            widgets.insert(instance_id.clone(), Owed { damage, commits: 1 });
            return;
        };
        owed.commits += 1;
        if owed.commits > MAX_OWED_COMMITS {
            owed.damage = WidgetDamage::Whole;
            return;
        }
        match (&mut owed.damage, damage) {
            (WidgetDamage::Regions(regions), WidgetDamage::Regions(more)) => regions.extend(more),
            (owed_damage, WidgetDamage::Whole) => *owed_damage = WidgetDamage::Whole,
            (WidgetDamage::Whole, WidgetDamage::Regions(_)) => {}
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Widgets(widgets) if widgets.is_empty())
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// What `instance_id` owes the next render, `None` under full damage or when it owes nothing.
    #[must_use]
    pub fn widget(&self, instance_id: &InstanceId) -> Option<&WidgetDamage> {
        match self {
            Self::Full => None,
            Self::Widgets(widgets) => widgets.get(instance_id).map(|owed| &owed.damage),
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

impl WidgetDamage {
    /// The damage a commit attached, as buffer rectangles.
    ///
    /// Surface-coordinate damage and a commit without damage both count as the whole buffer.
    #[must_use]
    pub fn of_commit(damage: &[Damage]) -> Self {
        let regions: Option<Vec<_>> = damage
            .iter()
            .map(|damage| match damage {
                Damage::Buffer(rect) => Some(*rect),
                Damage::Surface(_) => None,
            })
            .collect();
        match regions {
            Some(regions) if !regions.is_empty() => Self::Regions(regions),
            Some(_) | None => Self::Whole,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_OWED_COMMITS, OutputDamage, WidgetDamage};
    use bmc::compositor::InstanceId;
    use smithay::{
        utils::{Buffer, Logical, Rectangle},
        wayland::compositor::Damage,
    };

    fn id(name: &str) -> InstanceId {
        name.to_owned()
    }

    fn rect(x: i32, y: i32) -> Rectangle<i32, Buffer> {
        Rectangle::new((x, y).into(), (10, 10).into())
    }

    #[test]
    fn commits_before_a_render_add_up() {
        let mut damage = OutputDamage::default();
        damage.mark_widget(&id("clock"), WidgetDamage::Regions(vec![rect(0, 0)]));
        damage.mark_widget(&id("clock"), WidgetDamage::Regions(vec![rect(20, 0)]));

        assert_eq!(
            damage.widget(&id("clock")),
            Some(&WidgetDamage::Regions(vec![rect(0, 0), rect(20, 0)]))
        );
    }

    #[test]
    fn a_whole_buffer_commit_outweighs_regions_either_way() {
        let mut first = OutputDamage::default();
        first.mark_widget(&id("clock"), WidgetDamage::Whole);
        first.mark_widget(&id("clock"), WidgetDamage::Regions(vec![rect(0, 0)]));
        let mut second = OutputDamage::default();
        second.mark_widget(&id("clock"), WidgetDamage::Regions(vec![rect(0, 0)]));
        second.mark_widget(&id("clock"), WidgetDamage::Whole);

        assert_eq!(
            (first.widget(&id("clock")), second.widget(&id("clock"))),
            (Some(&WidgetDamage::Whole), Some(&WidgetDamage::Whole))
        );
    }

    #[test]
    fn many_regions_in_one_commit_stay_regions() {
        let mut damage = OutputDamage::default();
        let regions: Vec<_> = (0..64).map(|i| rect(i * 20, 0)).collect();
        damage.mark_widget(&id("clock"), WidgetDamage::Regions(regions.clone()));

        assert_eq!(
            damage.widget(&id("clock")),
            Some(&WidgetDamage::Regions(regions)),
            "a widget with many small changes keeps its regions"
        );
    }

    #[test]
    fn commits_piling_up_without_a_render_collapse_to_the_whole_buffer() {
        fn commit(damage: &mut OutputDamage, i: usize) {
            let i = i32::try_from(i).expect("BUG: the commit count is small");
            damage.mark_widget(&id("clock"), WidgetDamage::Regions(vec![rect(i, 0)]));
        }
        let mut damage = OutputDamage::default();
        for i in 0..MAX_OWED_COMMITS {
            commit(&mut damage, i);
        }
        let at_the_cap = matches!(damage.widget(&id("clock")), Some(WidgetDamage::Regions(_)));
        commit(&mut damage, MAX_OWED_COMMITS);

        assert_eq!(
            (at_the_cap, damage.widget(&id("clock"))),
            (true, Some(&WidgetDamage::Whole)),
            "the commit past the cap collapses, not the one reaching it"
        );
    }

    #[test]
    fn a_render_resets_the_commit_count() {
        let mut damage = OutputDamage::default();
        for _ in 0..MAX_OWED_COMMITS {
            damage.mark_widget(&id("clock"), WidgetDamage::Regions(vec![rect(0, 0)]));
        }
        damage.clear();
        damage.mark_widget(&id("clock"), WidgetDamage::Regions(vec![rect(0, 0)]));

        assert_eq!(
            damage.widget(&id("clock")),
            Some(&WidgetDamage::Regions(vec![rect(0, 0)]))
        );
    }

    #[test]
    fn full_damage_owes_no_widget_its_own_damage() {
        let mut before = OutputDamage::default();
        before.mark_widget(&id("clock"), WidgetDamage::Whole);
        before.mark_full();
        let mut after = OutputDamage::Full;
        after.mark_widget(&id("clock"), WidgetDamage::Whole);

        assert_eq!(
            (before.widget(&id("clock")), after.widget(&id("clock"))),
            (None, None)
        );
    }

    #[test]
    fn a_cleared_damage_owes_nothing() {
        let mut damage = OutputDamage::Full;
        damage.mark_widget(&id("clock"), WidgetDamage::Whole);
        damage.clear();

        assert!(damage.is_empty());
    }

    #[test]
    fn buffer_damage_keeps_its_rectangles() {
        assert_eq!(
            WidgetDamage::of_commit(&[Damage::Buffer(rect(5, 5))]),
            WidgetDamage::Regions(vec![rect(5, 5)])
        );
    }

    #[test]
    fn surface_damage_counts_as_the_whole_buffer() {
        let surface: Rectangle<i32, Logical> = Rectangle::new((0, 0).into(), (1, 1).into());

        assert_eq!(
            WidgetDamage::of_commit(&[Damage::Buffer(rect(5, 5)), Damage::Surface(surface)]),
            WidgetDamage::Whole
        );
    }

    #[test]
    fn a_commit_without_damage_counts_as_the_whole_buffer() {
        assert_eq!(WidgetDamage::of_commit(&[]), WidgetDamage::Whole);
    }
}
