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

//! Tracks the lifecycle state last emitted to each widget and produces
//! ordered release/acquire batches for the next emission.

use std::collections::HashMap;

use bmc::compositor::InstanceId;

use super::widget_tracker::LifecycleState;

/// Per-widget state transitions to emit, split into a release batch
/// (transitions into `Dormant`) and an acquire batch (transitions out
/// of `Dormant`). Transitions that keep the render target (neither
/// endpoint is `Dormant`) are placed in the acquire batch — they carry
/// no pool ordering requirement so either batch would do, and using
/// the acquire batch keeps the release batch focused on its single
/// purpose.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Emission {
    pub releases: Vec<(InstanceId, LifecycleState)>,
    pub acquires: Vec<(InstanceId, LifecycleState)>,
}

impl Emission {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.releases.is_empty() && self.acquires.is_empty()
    }
}

/// Keeps the last-emitted lifecycle state per widget and produces the
/// next emission as a release-then-acquire pair.
#[derive(Clone, Debug, Default)]
pub struct LifecycleEmitter {
    last: HashMap<InstanceId, LifecycleState>,
}

impl LifecycleEmitter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Compute the emission needed to move from the last-emitted state
    /// to `next`, and update the cached state. Widgets present in
    /// `self.last` but absent from `next` are treated as transitioning
    /// to `Dormant` (a widget that left the scene-cycling list goes
    /// off-screen by definition); the emitter then forgets them, since
    /// the corresponding `WidgetData` is gone on the protocol side.
    pub fn step(&mut self, next: &HashMap<InstanceId, LifecycleState>) -> Emission {
        let mut releases: Vec<(InstanceId, LifecycleState)> = Vec::new();
        let mut acquires: Vec<(InstanceId, LifecycleState)> = Vec::new();

        for (id, new_state) in next {
            let prev = self.last.insert(id.clone(), *new_state);
            if prev == Some(*new_state) {
                continue;
            }
            Self::guard_transition(id, prev, *new_state);
            if *new_state == LifecycleState::Dormant {
                if prev.is_none() {
                    continue;
                }
                releases.push((id.clone(), *new_state));
            } else {
                acquires.push((id.clone(), *new_state));
            }
        }

        let removed: Vec<InstanceId> = self
            .last
            .keys()
            .filter(|id| !next.contains_key(*id))
            .cloned()
            .collect();
        for id in removed {
            let prev = self.last.remove(&id);
            if prev != Some(LifecycleState::Dormant) {
                Self::guard_transition(&id, prev, LifecycleState::Dormant);
                releases.push((id, LifecycleState::Dormant));
            }
        }

        releases.sort_by(|a, b| a.0.cmp(&b.0));
        acquires.sort_by(|a, b| a.0.cmp(&b.0));

        Emission { releases, acquires }
    }

    fn guard_transition(id: &InstanceId, prev: Option<LifecycleState>, next: LifecycleState) {
        if !legal_transition(prev, next) {
            tracing::error!(
                instance = ?id, ?prev, next = ?next,
                "BUG: tracker produced XML-forbidden lifecycle transition; emitting anyway",
            );
            debug_assert!(
                false,
                "BUG: tracker produced XML-forbidden lifecycle transition {prev:?} -> {next:?} for instance {id:?}",
            );
        }
    }

    pub fn forget(&mut self, instance_id: &InstanceId) {
        self.last.remove(instance_id);
    }

    #[cfg(test)]
    #[must_use]
    pub fn last_state(&self, instance_id: &InstanceId) -> Option<LifecycleState> {
        self.last.get(instance_id).copied()
    }

    /// Record `state` as the last value emitted for `instance_id` without
    /// producing an emission. Used by the connect path (via
    /// [`CompositorState::send_initial_lifecycle`]), which sends the
    /// initial lifecycle event directly (outside [`Self::step`]); without
    /// this sync the next regular `step` would see `prev == None` and
    /// re-emit the same state.
    pub(super) fn record_initial(&mut self, instance_id: &InstanceId, state: LifecycleState) {
        self.last.insert(instance_id.clone(), state);
    }
}

#[must_use]
pub(crate) fn legal_transition(prev: Option<LifecycleState>, next: LifecycleState) -> bool {
    use LifecycleState::{Dormant, Entering, Leaving, Prepared, Visible};
    matches!(
        (prev, next),
        (None, _)
            | (Some(Dormant), Prepared | Visible | Entering)
            | (Some(Prepared), Dormant | Visible | Entering)
            | (Some(Visible), Dormant | Prepared | Leaving)
            | (Some(Entering | Leaving), Dormant | Prepared | Visible)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next(pairs: &[(&str, LifecycleState)]) -> HashMap<InstanceId, LifecycleState> {
        pairs.iter().map(|(id, s)| ((*id).to_owned(), *s)).collect()
    }

    #[test]
    fn first_step_emits_every_non_dormant_as_acquire() {
        let mut e = LifecycleEmitter::new();
        let em = e.step(&next(&[
            ("a", LifecycleState::Visible),
            ("b", LifecycleState::Dormant),
            ("c", LifecycleState::Prepared),
        ]));
        assert_eq!(em.releases, Vec::<(InstanceId, LifecycleState)>::new());
        assert_eq!(
            em.acquires,
            vec![
                (String::from("a"), LifecycleState::Visible),
                (String::from("c"), LifecycleState::Prepared),
            ],
        );
    }

    #[test]
    fn scene_swap_emits_release_then_acquire() {
        let mut e = LifecycleEmitter::new();
        let _ = e.step(&next(&[
            ("a", LifecycleState::Visible),
            ("b", LifecycleState::Prepared),
        ]));

        let em = e.step(&next(&[
            ("a", LifecycleState::Dormant),
            ("b", LifecycleState::Visible),
        ]));

        assert_eq!(
            em.releases,
            vec![(String::from("a"), LifecycleState::Dormant)]
        );
        assert_eq!(
            em.acquires,
            vec![(String::from("b"), LifecycleState::Visible)]
        );
    }

    #[test]
    fn keep_target_transitions_go_to_acquire_batch() {
        let mut e = LifecycleEmitter::new();
        let _ = e.step(&next(&[
            ("a", LifecycleState::Visible),
            ("b", LifecycleState::Prepared),
        ]));
        let em = e.step(&next(&[
            ("a", LifecycleState::Prepared),
            ("b", LifecycleState::Visible),
        ]));

        assert!(em.releases.is_empty());
        assert_eq!(
            em.acquires,
            vec![
                (String::from("a"), LifecycleState::Prepared),
                (String::from("b"), LifecycleState::Visible),
            ]
        );
    }

    #[test]
    fn unchanged_state_produces_empty_emission() {
        let mut e = LifecycleEmitter::new();
        let map = next(&[("a", LifecycleState::Visible)]);
        let _ = e.step(&map);
        let em = e.step(&map);
        assert!(em.is_empty());
    }

    #[test]
    fn removed_widget_emits_dormant_release_and_is_forgotten() {
        let mut e = LifecycleEmitter::new();
        let _ = e.step(&next(&[("a", LifecycleState::Visible)]));

        let em = e.step(&next(&[]));
        assert_eq!(
            em.releases,
            vec![(String::from("a"), LifecycleState::Dormant)]
        );
        assert!(em.acquires.is_empty());

        let em2 = e.step(&next(&[]));
        assert!(em2.is_empty());
    }

    #[test]
    fn record_initial_skips_re_emission_on_next_step() {
        // Connect path sends the initial event directly and syncs the
        // emitter via record_initial; the next scene step must not
        // re-emit the same state.
        let mut e = LifecycleEmitter::new();
        e.record_initial(&String::from("a"), LifecycleState::Visible);

        let em = e.step(&next(&[("a", LifecycleState::Visible)]));
        assert!(em.is_empty(), "no re-emit when cached state matches");

        let em = e.step(&next(&[("a", LifecycleState::Dormant)]));
        assert_eq!(
            em.releases,
            vec![(String::from("a"), LifecycleState::Dormant)]
        );
    }

    #[test]
    fn unrecorded_connect_emits_acquire_on_first_step() {
        // If the connect path could not send (no surface yet) and so
        // skipped record_initial, the next scene step is responsible
        // for delivering the lifecycle event as a regular acquire.
        let mut e = LifecycleEmitter::new();
        let em = e.step(&next(&[("a", LifecycleState::Visible)]));
        assert_eq!(
            em.acquires,
            vec![(String::from("a"), LifecycleState::Visible)]
        );
    }

    #[test]
    fn batches_are_sorted_by_instance_id() {
        let mut e = LifecycleEmitter::new();
        let em = e.step(&next(&[
            ("c", LifecycleState::Visible),
            ("a", LifecycleState::Prepared),
            ("b", LifecycleState::Entering),
        ]));
        assert_eq!(
            em.acquires
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
            vec![String::from("a"), String::from("b"), String::from("c"),],
        );
    }

    #[test]
    fn legal_transition_matches_xml_permits() {
        use LifecycleState::{Dormant, Entering, Leaving, Prepared, Visible};
        for s in [Dormant, Prepared, Visible, Entering, Leaving] {
            assert!(legal_transition(None, s), "initial {s:?} must be legal");
        }
        let permitted: &[(LifecycleState, &[LifecycleState])] = &[
            (Dormant, &[Prepared, Visible, Entering]),
            (Prepared, &[Dormant, Visible, Entering]),
            (Visible, &[Dormant, Prepared, Leaving]),
            (Entering, &[Visible, Prepared, Dormant]),
            (Leaving, &[Dormant, Prepared, Visible]),
        ];
        for (from, tos) in permitted {
            for to in *tos {
                assert!(
                    legal_transition(Some(*from), *to),
                    "{from:?} -> {to:?} must be legal"
                );
            }
            for to in [Dormant, Prepared, Visible, Entering, Leaving] {
                if to == *from || tos.contains(&to) {
                    continue;
                }
                assert!(
                    !legal_transition(Some(*from), to),
                    "{from:?} -> {to:?} must be forbidden"
                );
            }
        }
    }

    #[test]
    #[cfg_attr(not(debug_assertions), ignore = "debug_assert! is a no-op in release")]
    #[should_panic(expected = "XML-forbidden lifecycle transition")]
    fn step_debug_asserts_on_forbidden_emission() {
        let mut e = LifecycleEmitter::new();
        let _ = e.step(&next(&[("a", LifecycleState::Entering)]));
        let _ = e.step(&next(&[("a", LifecycleState::Leaving)]));
    }
}
