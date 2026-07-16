// Copyright (C) 2025  Braiins Systems s.r.o.
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

use anyhow::Result;
use bmc_button::{ButtonEventStream, ButtonId, Buttons, Edge};
use futures::StreamExt;
use futures::stream::empty;
use std::sync::Arc;

const BMC_RESET_BUTTON_NUMBER: u64 = 0;

#[derive(Clone, Debug)]
pub struct GeneralButton {
    pub button_number: u64,
    pub button_id: ButtonId,
    pub edge: Edge,
}

#[derive(Clone, Debug)]
pub struct GeneralButtons {
    pub list: Vec<GeneralButton>,
}

impl Buttons for GeneralButtons {
    fn to_stream(&self) -> Result<ButtonEventStream> {
        Ok(empty().boxed())
    }
}

#[must_use]
pub fn build_buttons() -> Arc<Box<dyn Buttons + Send + Sync + 'static>> {
    Arc::new(Box::new(GeneralButtons {
        list: vec![GeneralButton {
            button_number: BMC_RESET_BUTTON_NUMBER,
            button_id: ButtonId::Reset,
            edge: Edge::BothEdges,
        }],
    }))
}
