// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;

use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_grpc::web::{
    AddAlarmRequest, Alarm, AlarmInfoResponse, ListAlarmsResponse, Off, SetAlarmEnabledRequest,
    SnoozeOptionsWrapper, Weekday, alarm_service_server::AlarmService as GrpcAlarmService,
};
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

use crate::{alarm::AlarmController, config::ConfigHandle, sound::Sounds};

pub(crate) struct AlarmService {
    config_handle: Arc<RwLock<ConfigHandle>>,
    alarm_controller: AlarmController,
}

impl AlarmService {
    pub(crate) fn new(
        config_handle: Arc<RwLock<ConfigHandle>>,
        alarm_controller: AlarmController,
    ) -> Self {
        Self {
            config_handle,
            alarm_controller,
        }
    }
}

#[async_trait::async_trait]
impl GrpcAlarmService for AlarmService {
    async fn get_alarm_info(
        &self,
        request: Request<()>,
    ) -> Result<tonic::Response<AlarmInfoResponse>, Status> {
        todo!()
    }

    async fn list_alarms(
        &self,
        request: Request<()>,
    ) -> Result<Response<ListAlarmsResponse>, Status> {
        todo!()
    }

    async fn add_alarm(&self, request: Request<AddAlarmRequest>) -> Result<Response<()>, Status> {
        todo!()
    }

    async fn set_alarm(&self, request: Request<Alarm>) -> Result<Response<()>, Status> {
        todo!()
    }

    async fn delete_alarm(&self, request: Request<String>) -> Result<Response<()>, Status> {
        todo!()
    }

    async fn set_alarm_enabled(
        &self,
        request: Request<SetAlarmEnabledRequest>,
    ) -> Result<Response<()>, Status> {
        todo!()
    }
}
