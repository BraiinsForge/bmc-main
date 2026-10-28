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

//! A scene or widget looked up for an edit that may still be refused.

use crate::scene::{Scene, Widget, WidgetId};

/// Reading through it leaves the scenes clean for the next save;
/// only [`SceneEdit::into_mut`] marks them changed.
#[derive(Debug)]
pub struct SceneEdit<'a, T> {
    target: &'a mut T,
    scenes_dirty: &'a mut bool,
}

impl<'a, T> SceneEdit<'a, T> {
    pub(super) fn new(target: &'a mut T, scenes_dirty: &'a mut bool) -> Self {
        Self {
            target,
            scenes_dirty,
        }
    }

    pub fn get(&self) -> &T {
        self.target
    }

    pub fn into_mut(self) -> &'a mut T {
        *self.scenes_dirty = true;
        self.target
    }
}

impl<'a> SceneEdit<'a, Scene> {
    pub fn widget(self, id: &WidgetId) -> Option<SceneEdit<'a, Widget>> {
        let widget = self.target.widgets.get_mut(id)?;
        Some(SceneEdit::new(widget, self.scenes_dirty))
    }
}

#[cfg(test)]
mod tests {
    use crate::config::ConfigHandle;
    use crate::scene::{SceneId, WidgetId};

    async fn handle_with_a_widget() -> (tempfile::TempDir, ConfigHandle, SceneId, WidgetId) {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (handle, _) = ConfigHandle::init(
            dir.path().join("config.json"),
            80,
            30,
            80,
            30,
            bmc_platform::Product::Bmc100,
        )
        .await;
        let (scene_id, widget_id) = handle
            .scenes()
            .iter()
            .find_map(|(id, scene)| scene.widgets.keys().next().map(|widget| (*id, *widget)))
            .expect("BUG: the default scenes carry a widget");
        (dir, handle, scene_id, widget_id)
    }

    #[tokio::test]
    async fn a_refused_edit_leaves_the_scenes_clean() {
        let (_dir, mut handle, scene_id, widget_id) = handle_with_a_widget().await;
        let mut changes = handle.subscribe_scenes_change();

        let widget = handle
            .edit_scene(&scene_id)
            .and_then(|scene| scene.widget(&widget_id))
            .expect("BUG: the widget was just listed");
        assert_eq!(widget.get().id, widget_id);
        handle.save().await.expect("BUG: save");

        assert!(
            changes.try_recv().is_err(),
            "an edit that only read must not tell the widgets the scenes changed"
        );
    }

    #[tokio::test]
    async fn a_written_edit_reaches_the_next_save() {
        let (_dir, mut handle, scene_id, widget_id) = handle_with_a_widget().await;
        let mut changes = handle.subscribe_scenes_change();

        handle
            .edit_scene(&scene_id)
            .and_then(|scene| scene.widget(&widget_id))
            .expect("BUG: the widget was just listed")
            .into_mut()
            .params_version = Some(semver::Version::new(9, 0, 0));
        handle.save().await.expect("BUG: save");

        assert!(
            changes.try_recv().is_ok(),
            "a written edit must tell the widgets the scenes changed"
        );
    }
}
