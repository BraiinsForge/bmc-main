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

use crate::BmcManager;
use crate::led::LedController;
use bmc_grpc::web;
use bmc_led::data::{LedCommand, LedEffect, LedScene, Rgb};
use buffa::EnumValue;
use buffa_types::google::protobuf::Empty;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use std::time::Duration;

pub(crate) struct LedTestService<T: BmcManager> {
    led_controller: LedController<T>,
}

impl<T: BmcManager> LedTestService<T> {
    pub(crate) fn new(led_controller: LedController<T>) -> Self {
        Self { led_controller }
    }
}

impl<T: BmcManager> web::LedTestService for LedTestService<T> {
    async fn set_effect(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, web::SetLedEffectRequest>,
    ) -> ServiceResult<Empty> {
        let req = request.to_owned_message();
        let effect = map_effect(req.effect, req.color.into_option())?;
        let period_ms: u64 = req.period_ms.into();
        let scene = LedScene {
            effect,
            period: if period_ms > 0 {
                Some(Duration::from_millis(period_ms))
            } else {
                None
            },
            duration: if req.duration_ms > 0 {
                Some(Duration::from_millis(req.duration_ms.into()))
            } else {
                None
            },
        };
        self.led_controller
            .send_command(LedCommand::SetEffect(scene));
        Response::ok(Empty::default())
    }

    async fn set_brightness(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, web::SetLedBrightnessRequest>,
    ) -> ServiceResult<Empty> {
        self.led_controller
            .send_command(LedCommand::SetBrightness(request.brightness));
        Response::ok(Empty::default())
    }

    async fn disable(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        self.led_controller.send_command(LedCommand::Disable);
        Response::ok(Empty::default())
    }

    async fn enable(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        self.led_controller.send_command(LedCommand::Enable);
        Response::ok(Empty::default())
    }
}

fn map_effect(
    effect_type: EnumValue<web::LedEffectType>,
    color: Option<web::RgbColor>,
) -> Result<LedEffect, ConnectError> {
    let effect_type = match effect_type {
        EnumValue::Known(web::LedEffectType::Unspecified) | EnumValue::Unknown(_) => {
            return Err(ConnectError::invalid_argument("Effect type not specified"));
        }
        EnumValue::Known(known) => known,
    };
    if effect_type == web::LedEffectType::None {
        return Ok(LedEffect::None);
    }

    let color = color
        .ok_or_else(|| ConnectError::invalid_argument("Color is required for this effect type"))?;
    let rgb = Rgb::new(
        color.r.min(255) as u8,
        color.g.min(255) as u8,
        color.b.min(255) as u8,
    );

    match effect_type {
        web::LedEffectType::Chase => Ok(LedEffect::Chase(rgb)),
        web::LedEffectType::KnightRider => Ok(LedEffect::KnightRider(rgb)),
        web::LedEffectType::Scan => Ok(LedEffect::Scan(rgb)),
        web::LedEffectType::Snake => Ok(LedEffect::Snake(rgb)),
        web::LedEffectType::Breathe => Ok(LedEffect::Breathe(rgb)),
        web::LedEffectType::Solid => Ok(LedEffect::Solid(rgb)),
        web::LedEffectType::Unspecified | web::LedEffectType::None => unreachable!(),
    }
}
