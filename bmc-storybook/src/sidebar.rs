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

//! Filterable tree navigation sidebar.
//!
//! Builds a tree from `OwnedStoryGroupMeta` titles (slash-separated paths)
//! and `OwnedStoryEntry` items, rendered as nested `egui::CollapsingHeader`s.

use std::collections::BTreeMap;

use crate::hot_reload::{OwnedStoryEntry, OwnedStoryGroupMeta};

use crate::to_egui;

/// A node in the sidebar tree.
#[derive(Default, Clone)]
struct TreeNode {
    children: BTreeMap<String, TreeNode>,
    stories: Vec<usize>, // indices into the flat stories vec
}

/// Sidebar state.
pub struct SidebarState {
    tree: TreeNode,
    stories: Vec<OwnedStoryEntry>,
    groups: Vec<OwnedStoryGroupMeta>,
    pub filter: String,
    pub selected_idx: Option<usize>,
}

impl SidebarState {
    /// Build from owned entries and groups.
    pub fn new(entries: Vec<OwnedStoryEntry>, groups: Vec<OwnedStoryGroupMeta>) -> Self {
        let story_count = entries.len();
        let group_count = groups.len();
        tracing::info!(
            stories = story_count,
            groups = group_count,
            "sidebar: loaded"
        );

        let tree = build_tree(&entries, &groups);
        let selected_idx = if entries.is_empty() { None } else { Some(0) };

        Self {
            tree,
            stories: entries,
            groups,
            filter: String::new(),
            selected_idx,
        }
    }

    /// Replace stories and groups (hot-reload). Preserves selection by name.
    pub fn reload(&mut self, entries: Vec<OwnedStoryEntry>, groups: Vec<OwnedStoryGroupMeta>) {
        let selected_path = self.selected().map(|e| e.module_path.clone());
        let filter = std::mem::take(&mut self.filter);

        *self = Self::new(entries, groups);
        self.filter = filter;

        if let Some(path) = selected_path {
            self.select_by_module_path(&path);
        }
    }

    /// Get the currently selected story entry.
    #[must_use]
    pub fn selected(&self) -> Option<&OwnedStoryEntry> {
        self.selected_idx.map(|idx| &self.stories[idx])
    }

    /// Find the group title for a story entry.
    #[must_use]
    pub fn group_title_for(&self, entry: &OwnedStoryEntry) -> Option<&str> {
        self.groups
            .iter()
            .find(|m| entry.module_path.starts_with(&m.module_path))
            .map(|m| m.title.as_str())
    }

    /// Select a story by module path (unique across all stories). Returns true if found.
    pub fn select_by_module_path(&mut self, path: &str) -> bool {
        if let Some(idx) = self.stories.iter().position(|e| e.module_path == path) {
            self.selected_idx = Some(idx);
            true
        } else {
            false
        }
    }

    /// Whether the current filter matches any stories.
    pub fn has_visible_items(&self) -> bool {
        if self.filter.is_empty() {
            return !self.stories.is_empty();
        }
        node_matches_filter(&self.tree, &self.stories, "", &self.filter)
    }

    /// Render the sidebar tree. Returns true if selection changed.
    pub fn render(&mut self, ui: &mut egui::Ui, icons: &crate::icons::Icons) -> bool {
        let filter = self.filter.clone();
        let tree = self.tree.clone();
        let stories = &self.stories;
        let selected_idx = &mut self.selected_idx;
        render_tree_node(ui, &tree, stories, selected_idx, &filter, false, icons)
    }

    /// Get visible story indices in tree render order (respecting filter).
    fn visible_indices(&self) -> Vec<usize> {
        let mut indices = Vec::new();
        collect_visible_indices(&self.tree, &self.stories, &self.filter, false, &mut indices);
        indices
    }

    /// Select the next visible story (wraps around). Returns true if selection changed.
    pub fn select_next(&mut self) -> bool {
        let visible = self.visible_indices();
        if visible.is_empty() {
            return false;
        }
        let current_pos = self
            .selected_idx
            .and_then(|idx| visible.iter().position(|&i| i == idx));
        let next = match current_pos {
            Some(pos) => visible[(pos + 1) % visible.len()],
            None => visible[0],
        };
        let changed = self.selected_idx != Some(next);
        self.selected_idx = Some(next);
        changed
    }

    /// Select the previous visible story (wraps around). Returns true if selection changed.
    pub fn select_previous(&mut self) -> bool {
        let visible = self.visible_indices();
        if visible.is_empty() {
            return false;
        }
        let current_pos = self
            .selected_idx
            .and_then(|idx| visible.iter().position(|&i| i == idx));
        let prev = match current_pos {
            Some(0) | None => visible[visible.len() - 1],
            Some(pos) => visible[pos - 1],
        };
        let changed = self.selected_idx != Some(prev);
        self.selected_idx = Some(prev);
        changed
    }
}

/// Build the sidebar tree from entries and groups.
fn build_tree(stories: &[OwnedStoryEntry], groups: &[OwnedStoryGroupMeta]) -> TreeNode {
    let mut tree = TreeNode::default();

    // Build group paths from group metadata
    for meta in groups {
        let parts: Vec<&str> = meta.title.split('/').collect();
        let mut current = &mut tree;
        for part in parts {
            current = current.children.entry(part.to_owned()).or_default();
        }
    }

    // Place stories under their matching group by module path
    for (idx, entry) in stories.iter().enumerate() {
        let mut placed = false;
        for meta in groups {
            if entry.module_path.starts_with(&meta.module_path) {
                let parts: Vec<&str> = meta.title.split('/').collect();
                let mut current = &mut tree;
                for part in parts {
                    current = current.children.entry(part.to_owned()).or_default();
                }
                current.stories.push(idx);
                placed = true;
                break;
            }
        }
        if !placed {
            tree.stories.push(idx);
        }
    }

    sort_stories(&mut tree, stories);
    tree
}

/// Sort each node's stories by `(order, name)` so the catalog is deterministic;
/// inventory registration order is otherwise arbitrary link order.
fn sort_stories(node: &mut TreeNode, stories: &[OwnedStoryEntry]) {
    node.stories.sort_by(|&a, &b| {
        (stories[a].order, &stories[a].name).cmp(&(stories[b].order, &stories[b].name))
    });
    for child in node.children.values_mut() {
        sort_stories(child, stories);
    }
}

fn render_tree_node(
    ui: &mut egui::Ui,
    node: &TreeNode,
    stories: &[OwnedStoryEntry],
    selected_idx: &mut Option<usize>,
    filter: &str,
    // True when an ancestor directory's name already matched the filter,
    // so all descendant stories should be shown regardless of their own
    // name (without this flag, e.g. filtering "modal" matched the `Modal`
    // dir but then filtered out the stories inside that didn't contain
    // "modal" in their name, producing an empty dir).
    ancestor_matched: bool,
    icons: &crate::icons::Icons,
) -> bool {
    use crate::icons::icon_image;
    let mut changed = false;
    let filtering = !filter.is_empty();

    let folder_icon = &icons.folder;
    let folder_color = to_egui(bmc_render::colors::GOLD_60);

    let story_color = to_egui(bmc_render::colors::BLUE_60);
    let story_icon = &icons.app;

    let icon_size = 12.0;

    for (name, child) in &node.children {
        let name_matches = filtering && fuzzy_match(name, filter);
        if filtering
            && !ancestor_matched
            && !name_matches
            && !node_matches_filter(child, stories, name, filter)
        {
            continue;
        }
        let child_ancestor_matched = ancestor_matched || name_matches;

        // Default stories in single-leaf groups: render as a flat selectable
        // entry using the group name (e.g. "ProgressBar" instead of an
        // expandable header with one "Progress Bar Variants" child).
        let is_default_leaf = child.children.is_empty()
            && child.stories.len() == 1
            && stories[child.stories[0]].default;
        if is_default_leaf {
            let idx = child.stories[0];
            let is_selected = *selected_idx == Some(idx);
            let response = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.add(icon_image(story_icon, icon_size, story_color));
                    ui.selectable_label(is_selected, name)
                })
                .inner;
            if response.clicked() {
                *selected_idx = Some(idx);
                changed = true;
            }
        } else {
            let mut header = egui::CollapsingHeader::new(name);
            header = if filtering {
                header.open(Some(true))
            } else {
                header.default_open(false)
            };
            let resp = header.show(ui, |ui| {
                changed |= render_tree_node(
                    ui,
                    child,
                    stories,
                    selected_idx,
                    filter,
                    child_ancestor_matched,
                    icons,
                );
            });
            // Paint folder icon over the default triangle arrow.
            let hr = resp.header_response.rect;
            ui.painter().image(
                folder_icon.id(),
                egui::Rect::from_center_size(
                    egui::pos2(hr.left() + icon_size / 2.0 + 2.0, hr.center().y),
                    egui::vec2(icon_size, icon_size),
                ),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                folder_color,
            );
        }
    }

    for &idx in &node.stories {
        let entry = &stories[idx];
        if filtering && !ancestor_matched && !fuzzy_match(&entry.name, filter) {
            continue;
        }
        let is_selected = *selected_idx == Some(idx);
        let response = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add(icon_image(story_icon, icon_size, story_color));
                ui.selectable_label(is_selected, &entry.name)
            })
            .inner;
        if response.clicked() {
            *selected_idx = Some(idx);
            changed = true;
        }
    }

    changed
}

fn node_matches_filter(
    node: &TreeNode,
    stories: &[OwnedStoryEntry],
    name: &str,
    filter: &str,
) -> bool {
    if fuzzy_match(name, filter) {
        return true;
    }
    for &idx in &node.stories {
        if fuzzy_match(&stories[idx].name, filter) {
            return true;
        }
    }
    for (child_name, child) in &node.children {
        if node_matches_filter(child, stories, child_name, filter) {
            return true;
        }
    }
    false
}

/// Collect visible story indices in tree render order (children first, then stories).
fn collect_visible_indices(
    node: &TreeNode,
    stories: &[OwnedStoryEntry],
    filter: &str,
    ancestor_matched: bool,
    out: &mut Vec<usize>,
) {
    let filtering = !filter.is_empty();
    for (name, child) in &node.children {
        let name_matches = filtering && fuzzy_match(name, filter);
        if filtering
            && !ancestor_matched
            && !name_matches
            && !node_matches_filter(child, stories, name, filter)
        {
            continue;
        }
        collect_visible_indices(
            child,
            stories,
            filter,
            ancestor_matched || name_matches,
            out,
        );
    }
    for &idx in &node.stories {
        if filtering && !ancestor_matched && !fuzzy_match(&stories[idx].name, filter) {
            continue;
        }
        out.push(idx);
    }
}

/// Sublime Text-style fuzzy match.
fn fuzzy_match(text: &str, pattern: &str) -> bool {
    sublime_fuzzy::best_match(pattern, text).is_some()
}
