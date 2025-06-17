// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::DisplayConfigHandle;
use crate::web::grpc::GrpcError;
use bmc_display::data::{
    ClockAnalogRectConfig, ClockAnalogRoundConfig, ClockDigitalConfig, NumberFontStyle, Widget,
    WidgetType,
};
use bmc_grpc::web::{
    AddClockRequest, FontStyle, clock_style::Style,
    configuration_service_server::ConfigurationService as GrpcConfigurationService,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::Status;
use tonic_types::{ErrorDetails, FieldViolation, StatusExt};
use tracing::error;

pub(crate) struct ConfigurationService {
    display_config_handle: Arc<RwLock<DisplayConfigHandle>>,
}

impl ConfigurationService {
    pub(crate) fn new(display_config_handle: Arc<RwLock<DisplayConfigHandle>>) -> Self {
        Self {
            display_config_handle,
        }
    }
}

#[async_trait::async_trait]
impl GrpcConfigurationService for ConfigurationService {
    async fn add_clock_widget(
        &self,
        request: tonic::Request<AddClockRequest>,
    ) -> Result<tonic::Response<()>, Status> {
        let request = request.into_inner();
        let mut field_violations: Vec<FieldViolation> = vec![];

        if request.position.is_none() {
            field_violations.push(FieldViolation::new("position", "Position is not valid!"));
        }

        let number_font_style = try_from_font_style(request.number_font_style());
        if number_font_style.is_err() {
            field_violations.push(FieldViolation::new(
                "number_font_style",
                "Number Font Style is not valid!",
            ));
        }

        if let Some(clock_style) = request.clock_style {
            if clock_style.style.is_none() {
                field_violations.push(FieldViolation::new("style", "Style is not valid!"));
            }
        } else {
            field_violations.push(FieldViolation::new(
                "clock_style",
                "Clock style is not valid!",
            ));
        }

        if !field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(field_violations),
            ));
        }

        let Some(clock_style) = request.clock_style else {
            return Err(Status::invalid_argument("Invalid clock style"));
        };
        let Some(style) = clock_style.style else {
            return Err(Status::invalid_argument("Invalid style"));
        };

        let number_font_style = number_font_style?;
        let position = request
            .position
            .ok_or_else(|| Status::invalid_argument("Invalid position"))?;
        let timezone = request.timezone;

        let widget_type = parse_clock_style(style, number_font_style, timezone);

        let widget = Widget {
            // NOTE: u32 to i32 conversion of small integers should not fail
            row: position.row.try_into().unwrap_or_default(),
            col: position.col.try_into().unwrap_or_default(),
            widget_type,
        };

        {
            let mut config_handle = self.display_config_handle.write().await;
            let mut config_handle_cloned = config_handle.clone();
            config_handle_cloned.add_widget(None, widget);
            if let Err(e) = config_handle_cloned.sync_to_storage().await {
                error!("Cannot save display config: {}", e);
            } else {
                *config_handle = config_handle_cloned;
                // TODO: Update screen
            }
        }

        Ok(tonic::Response::new(()))
    }
}

fn try_from_font_style(font_style: FontStyle) -> Result<NumberFontStyle, Status> {
    let font_style = match font_style {
        FontStyle::FontstyleLight => NumberFontStyle::Light,
        FontStyle::FontstyleMedium => NumberFontStyle::Medium,
        FontStyle::FontstyleBold => NumberFontStyle::Bold,
        FontStyle::FontstyleUnspecified => {
            return Err(Status::invalid_argument("Invalid number font style"));
        }
    };

    Ok(font_style)
}

#[expect(clippy::too_many_lines)]
fn parse_clock_style(
    style: Style,
    number_font_style: NumberFontStyle,
    timezone: String,
) -> WidgetType {
    match style {
        // Analog Round
        bmc_grpc::web::clock_style::Style::ClockAnalogRoundS(config) => {
            WidgetType::ClockAnalogRoundSmall(ClockAnalogRoundConfig {
                show_date: false,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        bmc_grpc::web::clock_style::Style::ClockAnalogRoundM(config) => {
            WidgetType::ClockAnalogRoundMedium(ClockAnalogRoundConfig {
                show_date: false,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        bmc_grpc::web::clock_style::Style::ClockAnalogRoundL(config) => {
            WidgetType::ClockAnalogRoundLarge(ClockAnalogRoundConfig {
                show_date: false,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        bmc_grpc::web::clock_style::Style::ClockAnalogRoundF(config) => {
            WidgetType::ClockAnalogRoundFull(ClockAnalogRoundConfig {
                show_date: config.show_date,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        // Analog Rectangular
        bmc_grpc::web::clock_style::Style::ClockAnalogRectS(config) => {
            WidgetType::ClockAnalogRectSmall(ClockAnalogRectConfig {
                show_date: false,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        bmc_grpc::web::clock_style::Style::ClockAnalogRectM(config) => {
            WidgetType::ClockAnalogRectMedium(ClockAnalogRectConfig {
                show_date: false,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        bmc_grpc::web::clock_style::Style::ClockAnalogRectL(config) => {
            WidgetType::ClockAnalogRectLarge(ClockAnalogRectConfig {
                show_date: false,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        bmc_grpc::web::clock_style::Style::ClockAnalogRectF(config) => {
            WidgetType::ClockAnalogRectFull(ClockAnalogRectConfig {
                show_date: config.show_date,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        // Analog Digital
        bmc_grpc::web::clock_style::Style::ClockDigitalS(config) => {
            WidgetType::ClockDigitalSmall(ClockDigitalConfig {
                show_date: false,
                show_seconds: config.show_seconds,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        bmc_grpc::web::clock_style::Style::ClockDigitalM(config) => {
            WidgetType::ClockDigitalMedium(ClockDigitalConfig {
                show_date: false,
                show_seconds: config.show_seconds,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        bmc_grpc::web::clock_style::Style::ClockDigitalL(config) => {
            WidgetType::ClockDigitalLarge(ClockDigitalConfig {
                show_date: false,
                show_seconds: config.show_seconds,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
        bmc_grpc::web::clock_style::Style::ClockDigitalF(config) => {
            WidgetType::ClockDigitalFull(ClockDigitalConfig {
                show_date: config.show_date,
                show_seconds: config.show_seconds,
                show_timezone: config.show_timezone,
                number_font_style,
                timezone,
            })
        }
    }
}
