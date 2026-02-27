// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::led::LedController;
use bmc_grpc::web;
use bmc_led::data::{LedCommand, LedEffect, LedScene, Rgb};
use std::time::Duration;
use tonic::{Request, Response, Status};

pub(crate) struct LedTestService<T: BmcManager> {
    led_controller: LedController<T>,
}

impl<T: BmcManager> LedTestService<T> {
    pub(crate) fn new(led_controller: LedController<T>) -> Self {
        Self { led_controller }
    }
}

#[async_trait::async_trait]
impl<T: BmcManager> web::led_test_service_server::LedTestService for LedTestService<T> {
    async fn set_effect(
        &self,
        request: Request<web::SetLedEffectRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let effect = map_effect(req.effect(), req.color)?;
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
        Ok(Response::new(()))
    }

    async fn set_brightness(
        &self,
        request: Request<web::SetLedBrightnessRequest>,
    ) -> Result<Response<()>, Status> {
        let brightness = request.into_inner().brightness;
        self.led_controller
            .send_command(LedCommand::SetBrightness(brightness));
        Ok(Response::new(()))
    }

    async fn disable(&self, _request: Request<()>) -> Result<Response<()>, Status> {
        self.led_controller.send_command(LedCommand::Disable);
        Ok(Response::new(()))
    }

    async fn enable(&self, _request: Request<()>) -> Result<Response<()>, Status> {
        self.led_controller.send_command(LedCommand::Enable);
        Ok(Response::new(()))
    }
}

fn map_effect(
    effect_type: web::LedEffectType,
    color: Option<web::RgbColor>,
) -> Result<LedEffect, Status> {
    if effect_type == web::LedEffectType::Unspecified {
        return Err(Status::invalid_argument("Effect type not specified"));
    }
    if effect_type == web::LedEffectType::None {
        return Ok(LedEffect::None);
    }

    let color =
        color.ok_or_else(|| Status::invalid_argument("Color is required for this effect type"))?;
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
