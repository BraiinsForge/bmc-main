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

use super::super::*;
use super::{grpc, grpc_web, transport, violation_fields};
use crate::backlight::DisplayBacklightDriver;
use crate::bootloader_config::BootloaderConfig;
use crate::compositor::SettingUpdate;
use crate::compositor::testing::RecordingCompositor;
use crate::session;
use crate::system_upgrade::boser::UpgradeRoute;
use crate::{App, BmcManager, Configuration, UpgradeError, UpgradeMarker};
use axum_extra::extract::cookie::Cookie;
use bmc_button::{ButtonEventStream, Buttons};
use bmc_grpc::web::{
    ChangePasswordRequest, CheckForUpgradeRequest, CheckForUpgradeResponse, CreatePasswordRequest,
    GetAutoUpgradeResponse, GetInstallableWidgetsResponse, GetTimezoneListResponse,
    GetTimezoneResponse, InitialSetupServiceClient, NetworkConfig, NetworkConfigStatic,
    NetworkInfoResponse, NetworkService, NetworkServiceClient, RemovePasswordRequest,
    ScanWifiResponse, SetAutoUpgradeRequest, SetTimezoneRequest, SetWifiRequest, SettingsRequest,
    StartUpgradeRequest, SystemService, SystemServiceClient, UpgradeProgress, UpgradeService,
    UpgradeServiceClient, WifiSavedNetworksResponse, WifiStatusResponse, network_config::Protocol,
};
use bmc_led::led_driver::LedDriver;
use bmc_net::mock::MockNetworkManager;
use bmc_platform::{BosPlatform, BosVersion, HardwareProfile, Product};
use bmc_shared_time::time::Timezone;
use bmc_upgrade::firmware::{FirmwareDownloadError, FirmwareIndex, UpgradeMetadata};
use bmc_upgrade::packages::{
    ApplyError, EstimateMode, InstallablePackage, PackageBackend, PackageGcError, PackageGcOutcome,
    PackageGcRequest, PackageProbe, PackageProbeError,
};
use buffa_types::google::protobuf::{BoolValue, Empty};
use connectrpc::client::ServiceTransport;
use connectrpc::{
    ErrorCode, RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream,
};
use reqwest::Client;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, watch};

const UNREACHABLE: &str = "BUG: session manager is not called by these tests";
const GATED_ROUTE_UNREACHABLE: &str = "BUG: gated production route reached its stub";

#[derive(Clone, Debug)]
pub(super) struct StubSession;

impl session::Handle for StubSession {
    fn is_valid(&self) -> bool {
        true
    }

    fn id(&self) -> String {
        "test-session".to_owned()
    }
}

#[derive(Debug, Default)]
pub(super) struct StubSessionManager;

#[async_trait::async_trait]
impl session::Manager for StubSessionManager {
    type Error = std::io::Error;
    type Session = StubSession;

    const SESSION_TIMEOUT: u32 = 0;

    async fn login(&self, _password: &str) -> Result<Cookie<'static>, Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }

    async fn logout(&self, _session: Self::Session) -> Result<Cookie<'static>, Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }

    async fn logout_all_related(&self, _session: Self::Session) -> Result<(), Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }

    async fn extend(&self, _session: Self::Session) -> Result<Cookie<'static>, Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }

    async fn find(&self, _cookies: &[Cookie<'_>]) -> Result<Self::Session, Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }
}

#[derive(Debug)]
struct StubFirmwareIndex;

#[async_trait::async_trait]
impl FirmwareIndex for StubFirmwareIndex {
    async fn get_available_releases(
        &self,
        _client: &Client,
        _platform: BosPlatform,
        _version: String,
    ) -> Result<Option<Vec<UpgradeMetadata>>, FirmwareDownloadError> {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }
}

#[derive(Debug)]
struct StubPackageBackend;

#[async_trait::async_trait]
impl PackageBackend for StubPackageBackend {
    async fn gc(&self, _request: PackageGcRequest) -> Result<PackageGcOutcome, PackageGcError> {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }

    async fn probe(
        &self,
        _firmware: Option<&str>,
        _estimate: EstimateMode,
        _install: &[String],
    ) -> PackageProbe {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }

    async fn apply(
        &self,
        _merged: bmc_nix::types::MergedIndex,
        _install: Vec<String>,
        _progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
    ) -> Result<(), ApplyError> {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }

    async fn list_installable_packages(
        &self,
        _firmware: Option<&str>,
    ) -> Result<Vec<InstallablePackage>, PackageProbeError> {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }

    fn store_free_bytes(&self) -> std::io::Result<u64> {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }
}

#[derive(Clone, Debug)]
struct StubBacklightDriver;

impl DisplayBacklightDriver for StubBacklightDriver {
    fn init(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn change_state(&self, _enabled: bool) -> anyhow::Result<()> {
        Ok(())
    }

    fn state(&self) -> anyhow::Result<bool> {
        Ok(true)
    }

    fn brightness(&self) -> anyhow::Result<u8> {
        Ok(100)
    }

    fn max_brightness(&self) -> u8 {
        u8::MAX
    }

    fn set_brightness(&self, _value: u8) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct StubButtons;

impl Buttons for StubButtons {
    fn to_stream(&self) -> anyhow::Result<ButtonEventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }
}

#[derive(Debug)]
pub(super) struct StubBmcManager {
    network_manager: MockNetworkManager,
    timezone_sender: watch::Sender<Timezone>,
}

impl StubBmcManager {
    pub(super) fn with_network(network_manager: MockNetworkManager) -> Self {
        Self {
            network_manager,
            timezone_sender: watch::channel(Timezone::default()).0,
        }
    }
}

impl Default for StubBmcManager {
    fn default() -> Self {
        Self::with_network(MockNetworkManager::with_provisioning(false, true))
    }
}

#[async_trait::async_trait]
impl BmcManager for StubBmcManager {
    type SessionManager = StubSessionManager;
    type Error = std::io::Error;

    async fn version(&self) -> Option<BosVersion> {
        Some(BosVersion::new(&"current", &"current"))
    }

    fn platform(&self) -> BosPlatform {
        BosPlatform::Bfm1
    }

    fn network_manager(&self) -> &dyn bmc_net::NetworkManager {
        &self.network_manager
    }

    async fn upgrade(
        &self,
        _keep_settings: bool,
        _upgrade_image_path: &Path,
        _progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(), UpgradeError> {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }

    async fn consume_upgrade_marker(&self) -> UpgradeMarker {
        UpgradeMarker::Absent
    }

    async fn consume_service_upgrade_marker(&self) -> UpgradeMarker {
        UpgradeMarker::Absent
    }

    fn session_manager(&self) -> Self::SessionManager {
        StubSessionManager
    }

    async fn check_password(&self, _password: Option<&str>) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn set_password(&self, _password: Option<String>) -> Result<(), Self::Error> {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }

    fn timezone(&self) -> Timezone {
        self.timezone_sender.borrow().clone()
    }

    fn publish_timezone(&self, _timezone: Timezone) -> bool {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }

    async fn set_timezone(&self, _timezone: Timezone) -> anyhow::Result<()> {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }

    fn watch_timezone_updates(&self) -> watch::Receiver<Timezone> {
        self.timezone_sender.subscribe()
    }

    async fn factory_reset(&self, _hard: bool) -> Result<(), Self::Error> {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }

    async fn reboot(&self) -> anyhow::Result<()> {
        unimplemented!("{GATED_ROUTE_UNREACHABLE}")
    }

    async fn handle_graceful_shutdown(&self) {}

    fn support_archive(&self) -> impl tokio::io::AsyncRead + Send + Unpin + 'static {
        tokio::io::empty()
    }

    async fn sync_boot_environment(&self, _config: &BootloaderConfig) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn control_service(&self, _service: &str, _actions: &[&str]) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Default)]
struct RecordingSystemService {
    calls: Arc<AtomicUsize>,
}

impl RecordingSystemService {
    fn record(&self) -> ServiceResult<Empty> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Response::ok(Empty::default())
    }
}

impl SystemService for RecordingSystemService {
    async fn has_password(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<BoolValue> {
        Response::ok(BoolValue::default())
    }

    async fn create_password(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, CreatePasswordRequest>,
    ) -> ServiceResult<Empty> {
        self.record()
    }

    async fn change_password(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ChangePasswordRequest>,
    ) -> ServiceResult<Empty> {
        self.record()
    }

    async fn remove_password(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, RemovePasswordRequest>,
    ) -> ServiceResult<Empty> {
        self.record()
    }

    async fn get_timezone(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<GetTimezoneResponse> {
        Response::ok(GetTimezoneResponse::default())
    }

    async fn set_timezone(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, SetTimezoneRequest>,
    ) -> ServiceResult<Empty> {
        self.record()
    }

    async fn get_timezone_list(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<GetTimezoneListResponse> {
        Response::ok(GetTimezoneListResponse::default())
    }

    async fn factory_reset(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        self.record()
    }

    async fn reboot(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        self.record()
    }
}

#[derive(Clone, Default)]
struct RecordingNetworkService {
    calls: Arc<AtomicUsize>,
}

impl RecordingNetworkService {
    fn record(&self) -> ServiceResult<Empty> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Response::ok(Empty::default())
    }
}

impl NetworkService for RecordingNetworkService {
    async fn get_network_info(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<NetworkInfoResponse> {
        Response::ok(NetworkInfoResponse::default())
    }

    async fn get_network_config(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<NetworkConfig> {
        Response::ok(NetworkConfig::default())
    }

    async fn set_network_config(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, NetworkConfig>,
    ) -> ServiceResult<Empty> {
        self.record()
    }

    async fn get_wifi_status(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<WifiStatusResponse> {
        Response::ok(WifiStatusResponse::default())
    }

    async fn get_wifi_saved_networks(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<WifiSavedNetworksResponse> {
        Response::ok(WifiSavedNetworksResponse::default())
    }

    async fn set_wifi(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, SetWifiRequest>,
    ) -> ServiceResult<Empty> {
        self.record()
    }

    async fn scan_wifi(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ScanWifiResponse> {
        Response::ok(ScanWifiResponse::default())
    }
}

#[derive(Clone, Default)]
struct RecordingUpgradeService;

impl UpgradeService for RecordingUpgradeService {
    async fn check_for_upgrade(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, CheckForUpgradeRequest>,
    ) -> ServiceResult<CheckForUpgradeResponse> {
        Response::ok(CheckForUpgradeResponse::default())
    }

    async fn get_installable_widgets(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<GetInstallableWidgetsResponse> {
        Response::ok(GetInstallableWidgetsResponse::default())
    }

    async fn start_upgrade(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, StartUpgradeRequest>,
    ) -> ServiceResult<ServiceStream<UpgradeProgress>> {
        Ok(Response::stream(futures::stream::empty()))
    }

    async fn set_auto_upgrade(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, SetAutoUpgradeRequest>,
    ) -> ServiceResult<Empty> {
        Response::ok(Empty::default())
    }

    async fn get_auto_upgrade(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<GetAutoUpgradeResponse> {
        Response::ok(GetAutoUpgradeResponse::default())
    }
}

fn capabilities(product: Product) -> HardwareCapabilities {
    HardwareProfile::for_product(product).capabilities()
}

/// The three services Boser may own, served by recording stubs.
fn owned_services(system: RecordingSystemService, network: RecordingNetworkService) -> Router {
    Router::new()
        .add_service(Arc::new(system))
        .add_service(Arc::new(network))
        .add_service(Arc::new(RecordingUpgradeService))
}

fn gated_router(product: Product, router: Router) -> axum::Router {
    gated(router, Arc::new(StubSessionManager), capabilities(product))
}

async fn stub_session(mut request: Request) -> Request {
    request.extensions_mut().insert(StubSession);
    request
}

/// `router` as the session layer leaves it for a logged-in caller.
pub(super) fn with_session(router: axum::Router) -> axum::Router {
    router.layer(axum::middleware::map_request(stub_session))
}

/// A session a gate check reads, as the session layer would have left it.
fn session_extensions() -> http::Extensions {
    let mut extensions = http::Extensions::new();
    extensions.insert(StubSession);
    extensions
}

fn system_client(router: &axum::Router) -> SystemServiceClient<ServiceTransport<axum::Router>> {
    SystemServiceClient::new(transport(router.clone()), grpc())
}

fn network_client(router: &axum::Router) -> NetworkServiceClient<ServiceTransport<axum::Router>> {
    NetworkServiceClient::new(transport(router.clone()), grpc())
}

pub(super) fn upgrade_client(
    router: &axum::Router,
) -> UpgradeServiceClient<ServiceTransport<axum::Router>> {
    UpgradeServiceClient::new(transport(router.clone()), grpc())
}

/// The procedures `router` serves under `service_name`, with their leading slash.
fn service_method_paths(router: &Router, service_name: &str) -> Vec<String> {
    let prefix = format!("{service_name}/");
    router
        .methods()
        .filter(|method| method.starts_with(&prefix))
        .map(|method| format!("/{method}"))
        .collect()
}

async fn production_router(product: Product) -> (tempfile::TempDir, axum::Router) {
    let (tempdir, app) = production_app(
        Arc::new(StubBmcManager::default()),
        Arc::new(RecordingCompositor::with_hardware_capabilities(
            capabilities(product),
        )),
    )
    .await;
    (tempdir, app.build_grpc_router())
}

/// The production router for `product` with Boser at `boser`;
/// `route` replaces the upgrade route startup chose, so a test can publish the display by hand.
pub(super) async fn upgrade_router(
    product: Product,
    boser: Option<SocketAddr>,
    route: Option<UpgradeRoute>,
) -> (tempfile::TempDir, axum::Router) {
    let (tempdir, app) = production_app_with_boser(
        Arc::new(StubBmcManager::default()),
        Arc::new(RecordingCompositor::with_hardware_capabilities(
            capabilities(product),
        )),
        boser,
    )
    .await;
    let app = match route {
        Some(route) => app.with_upgrade_route(route),
        None => app,
    };
    (tempdir, app.build_grpc_router())
}

async fn production_app(
    manager: Arc<StubBmcManager>,
    compositor: Arc<RecordingCompositor>,
) -> (
    tempfile::TempDir,
    App<StubBmcManager, StubBacklightDriver, StubFirmwareIndex>,
) {
    production_app_with_boser(manager, compositor, None).await
}

async fn production_app_with_boser(
    manager: Arc<StubBmcManager>,
    compositor: Arc<RecordingCompositor>,
    boser: Option<SocketAddr>,
) -> (
    tempfile::TempDir,
    App<StubBmcManager, StubBacklightDriver, StubFirmwareIndex>,
) {
    let tempdir = tempfile::tempdir().expect("BUG: test tempdir creation must succeed");
    let mut config = Configuration {
        address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        upgrade_image_path: tempdir.path().join("firmware.tar"),
        config_path: tempdir.path().join("config.json"),
        sounds_dir: tempdir.path().join("sounds"),
        crontab_path: None,
        nix_servers_config_path: tempdir.path().join("servers.json"),
        nix_gc_config_path: tempdir.path().join("gc.json"),
        nix_profile_dir: tempdir.path().join("profile"),
        pending_install_path: tempdir.path().join("pending-install.json"),
        ..Configuration::default()
    };
    config.server_config.boser = boser;
    // Keeps the test off the host's token file.
    config.boser_token_path = tempdir.path().join("no-boser-token");
    let (command_sender, _command_receiver) = tokio::sync::mpsc::channel(4);
    let app = App::init(
        config,
        manager,
        StubSessionManager,
        Arc::new(Mutex::new(StubBacklightDriver)),
        LedDriver { command_sender },
        StubFirmwareIndex,
        Arc::new(StubPackageBackend),
        Arc::new(Box::new(StubButtons)),
        compositor,
        None,
    )
    .await
    .expect("managed test application must initialize");

    (tempdir, app)
}

#[tokio::test]
async fn timezone_changes_reach_widgets_without_alarm_support() {
    let manager = Arc::new(StubBmcManager::default());
    let compositor = Arc::new(RecordingCompositor::with_hardware_capabilities(
        capabilities(Product::Bmm101),
    ));
    assert!(
        !capabilities(Product::Bmm101).alarm_supported,
        "the test needs a product whose alarm backend is unsupported"
    );
    let (_tempdir, app) = production_app(Arc::clone(&manager), Arc::clone(&compositor)).await;
    let next_alarm = app.subscribe_next_alarm();
    let _server = app.into_idle_server();
    assert!(
        next_alarm.has_changed().is_ok(),
        "the settings listener exits once the next-alarm channel closes"
    );

    let prague = Timezone::from_str("Europe/Prague").expect("BUG: valid test timezone");
    manager.timezone_sender.send_replace(prague);

    compositor
        .wait_for_setting(&SettingUpdate::Timezone("Europe/Prague".to_owned()))
        .await;
}

async fn call_boser_owned_mutations(router: &axum::Router) -> Vec<Result<(), ConnectError>> {
    let system = system_client(router);
    let network = network_client(router);

    vec![
        system
            .create_password(CreatePasswordRequest::default())
            .await
            .map(|_| ()),
        system
            .change_password(ChangePasswordRequest::default())
            .await
            .map(|_| ()),
        system
            .remove_password(RemovePasswordRequest::default())
            .await
            .map(|_| ()),
        system
            .set_timezone(SetTimezoneRequest::default())
            .await
            .map(|_| ()),
        system.factory_reset(Empty::default()).await.map(|_| ()),
        system.reboot(Empty::default()).await.map(|_| ()),
        network
            .set_network_config(NetworkConfig::default())
            .await
            .map(|_| ()),
        network
            .set_wifi(SetWifiRequest::default())
            .await
            .map(|_| ()),
    ]
}

fn assert_boser_managed(error: &ConnectError) {
    assert_eq!(error.code, ErrorCode::Unimplemented);
    assert_eq!(error.message.as_deref(), Some(BOSER_MANAGED_STATUS_MESSAGE));
}

#[tokio::test]
async fn production_routes_gate_system_and_network_mutations() {
    let (_tempdir, router) = production_router(Product::Bfm100).await;
    let router = with_session(router);
    let system = system_client(&router);
    let network = network_client(&router);

    let results = [
        system.reboot(Empty::default()).await.map(|_| ()),
        network
            .set_wifi(SetWifiRequest::default())
            .await
            .map(|_| ()),
    ];

    for result in results {
        let error = result.expect_err("managed production route must reject mutation");
        assert_boser_managed(&error);
    }
}

#[tokio::test]
async fn production_routes_preserve_self_managed_network_configuration() {
    let (_tempdir, router) = production_router(Product::Bmc100).await;
    let network = network_client(&with_session(router));

    network
        .set_network_config(NetworkConfig {
            protocol: Some(Protocol::Dhcp(Empty::default())),
        })
        .await
        .expect("BUG: self-managed production route must reach the network handler");
}

#[tokio::test]
async fn field_violations_reach_clients_over_grpc_and_grpc_web() {
    let (_tempdir, router) = production_router(Product::Bmc100).await;
    let router = with_session(router);
    let invalid_address = NetworkConfig {
        protocol: Some(Protocol::Static(NetworkConfigStatic {
            address: "not-an-address".to_owned(),
            netmask: "255.255.255.0".to_owned(),
            gateway: "192.168.1.1".to_owned(),
            dns_servers: vec!["192.168.1.1".to_owned()],
        })),
    };

    for (protocol, config) in [("gRPC", grpc()), ("gRPC-Web", grpc_web())] {
        let error = NetworkServiceClient::new(transport(router.clone()), config)
            .set_network_config(invalid_address.clone())
            .await
            .expect_err("an invalid address is refused");

        assert_eq!(error.code, ErrorCode::InvalidArgument, "{protocol}");
        assert_eq!(violation_fields(&error), ["address"], "{protocol}");
    }
}

#[tokio::test]
async fn managed_production_routes_keep_initial_setup_available() {
    let (_tempdir, router) = production_router(Product::Bfm100).await;
    let initial_setup = InitialSetupServiceClient::new(transport(router), grpc());

    let settings = initial_setup
        .get_settings_data(Empty::default())
        .await
        .expect("managed production route must reach the initial-setup handler")
        .into_owned();

    assert!(
        !settings.timezones.is_empty(),
        "initial setup must return its timezone choices"
    );

    let error = initial_setup
        .setup_device(SettingsRequest::default())
        .await
        .expect_err("an invalid setup request must reach field validation");
    assert_eq!(error.code, ErrorCode::InvalidArgument);
}

/// Boser owns the automatic-upgrade preference.
#[tokio::test]
async fn managed_production_routes_reject_the_auto_upgrade_rpcs() {
    let (_tempdir, router) = production_router(Product::Bfm100).await;
    let upgrade = upgrade_client(&with_session(router));

    let results = [
        upgrade
            .set_auto_upgrade(SetAutoUpgradeRequest::default())
            .await
            .map(|_| ()),
        upgrade.get_auto_upgrade(Empty::default()).await.map(|_| ()),
    ];

    for result in results {
        let error = result.expect_err("managed auto-upgrade RPC must be rejected");
        assert_boser_managed(&error);
    }
}

#[tokio::test]
async fn self_managed_production_routes_keep_upgrade_available() {
    let (_tempdir, router) = production_router(Product::Bmc100).await;
    let upgrade = upgrade_client(&with_session(router));

    upgrade
        .get_auto_upgrade(Empty::default())
        .await
        .expect("self-managed production route must reach the upgrade handler");
}

#[tokio::test]
async fn managed_mutations_are_rejected_before_handlers() {
    let system = RecordingSystemService::default();
    let network = RecordingNetworkService::default();
    let calls = [system.calls.clone(), network.calls.clone()];
    let router = with_session(gated_router(
        Product::Bfm100,
        owned_services(system, network),
    ));

    let results = call_boser_owned_mutations(&router).await;

    assert_eq!(results.len(), 8);
    for result in results {
        let error = result.expect_err("managed mutation must be rejected");
        assert_boser_managed(&error);
    }
    assert_eq!(
        calls
            .iter()
            .map(|count| count.load(Ordering::Relaxed))
            .sum::<usize>(),
        0
    );
}

#[tokio::test]
async fn self_managed_mutations_reach_handlers() {
    let system = RecordingSystemService::default();
    let network = RecordingNetworkService::default();
    let calls = [system.calls.clone(), network.calls.clone()];
    let router = with_session(gated_router(
        Product::Bmc100,
        owned_services(system, network),
    ));

    let results = call_boser_owned_mutations(&router).await;

    for result in results {
        result.expect("self-managed mutation must reach its handler");
    }
    assert_eq!(
        calls
            .iter()
            .map(|count| count.load(Ordering::Relaxed))
            .sum::<usize>(),
        8
    );
}

#[tokio::test]
async fn authentication_precedes_ownership_check() {
    let router = gated_router(
        Product::Bfm100,
        owned_services(
            RecordingSystemService::default(),
            RecordingNetworkService::default(),
        ),
    );

    let error = system_client(&router)
        .reboot(Empty::default())
        .await
        .expect_err("missing session must be rejected first");

    assert_eq!(error.code, ErrorCode::Unauthenticated);
}

#[tokio::test]
async fn a_bmc_owned_call_passes_the_gate_on_a_managed_platform() {
    let router = with_session(gated_router(
        Product::Bfm100,
        owned_services(
            RecordingSystemService::default(),
            RecordingNetworkService::default(),
        ),
    ));

    system_client(&router)
        .has_password(Empty::default())
        .await
        .expect("a BMC-owned method stays available");
}

const EXPECTED_MANAGED_RPC_OWNERS: [(&str, &str, ManagedRpcOwner); 21] = [
    (
        web::SYSTEM_SERVICE_SERVICE_NAME,
        "HasPassword",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::SYSTEM_SERVICE_SERVICE_NAME,
        "CreatePassword",
        ManagedRpcOwner::Boser,
    ),
    (
        web::SYSTEM_SERVICE_SERVICE_NAME,
        "ChangePassword",
        ManagedRpcOwner::Boser,
    ),
    (
        web::SYSTEM_SERVICE_SERVICE_NAME,
        "RemovePassword",
        ManagedRpcOwner::Boser,
    ),
    (
        web::SYSTEM_SERVICE_SERVICE_NAME,
        "GetTimezone",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::SYSTEM_SERVICE_SERVICE_NAME,
        "SetTimezone",
        ManagedRpcOwner::Boser,
    ),
    (
        web::SYSTEM_SERVICE_SERVICE_NAME,
        "GetTimezoneList",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::SYSTEM_SERVICE_SERVICE_NAME,
        "FactoryReset",
        ManagedRpcOwner::Boser,
    ),
    (
        web::SYSTEM_SERVICE_SERVICE_NAME,
        "Reboot",
        ManagedRpcOwner::Boser,
    ),
    (
        web::NETWORK_SERVICE_SERVICE_NAME,
        "GetNetworkInfo",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::NETWORK_SERVICE_SERVICE_NAME,
        "GetNetworkConfig",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::NETWORK_SERVICE_SERVICE_NAME,
        "SetNetworkConfig",
        ManagedRpcOwner::Boser,
    ),
    (
        web::NETWORK_SERVICE_SERVICE_NAME,
        "GetWifiStatus",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::NETWORK_SERVICE_SERVICE_NAME,
        "GetWifiSavedNetworks",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::NETWORK_SERVICE_SERVICE_NAME,
        "SetWifi",
        ManagedRpcOwner::Boser,
    ),
    (
        web::NETWORK_SERVICE_SERVICE_NAME,
        "ScanWifi",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::UPGRADE_SERVICE_SERVICE_NAME,
        "CheckForUpgrade",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::UPGRADE_SERVICE_SERVICE_NAME,
        "GetInstallableWidgets",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::UPGRADE_SERVICE_SERVICE_NAME,
        "StartUpgrade",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::UPGRADE_SERVICE_SERVICE_NAME,
        "SetAutoUpgrade",
        ManagedRpcOwner::Boser,
    ),
    (
        web::UPGRADE_SERVICE_SERVICE_NAME,
        "GetAutoUpgrade",
        ManagedRpcOwner::Boser,
    ),
];

#[test]
fn every_ownership_gated_service_method_has_the_expected_owner() {
    let router = owned_services(
        RecordingSystemService::default(),
        RecordingNetworkService::default(),
    );
    let mut actual_paths = [
        web::SYSTEM_SERVICE_SERVICE_NAME,
        web::NETWORK_SERVICE_SERVICE_NAME,
        web::UPGRADE_SERVICE_SERVICE_NAME,
    ]
    .into_iter()
    .flat_map(|service| service_method_paths(&router, service))
    .collect::<Vec<_>>();
    let mut expected_paths = EXPECTED_MANAGED_RPC_OWNERS
        .iter()
        .map(|(service, method, _)| format!("/{service}/{method}"))
        .collect::<Vec<_>>();
    actual_paths.sort_unstable();
    expected_paths.sort_unstable();
    assert_eq!(actual_paths, expected_paths);

    for (service, method, expected_owner) in EXPECTED_MANAGED_RPC_OWNERS {
        let path = format!("/{service}/{method}");
        assert_eq!(managed_rpc_owner(&path), Some(expected_owner), "{path}");
    }
}

#[test]
fn the_gate_enforces_every_managed_rpc_owner() {
    let gate = AccessGate {
        router: Arc::new(owned_services(
            RecordingSystemService::default(),
            RecordingNetworkService::default(),
        )),
        session_manager: Arc::new(StubSessionManager),
        hardware_capabilities: capabilities(Product::Bfm100),
    };
    let session = session_extensions();

    for (service, method, expected_owner) in EXPECTED_MANAGED_RPC_OWNERS {
        let path = format!("/{service}/{method}");
        let result = gate.check(&path, &session);
        match expected_owner {
            ManagedRpcOwner::Bmc => {
                result.expect("BMC-owned operation must remain available");
            }
            ManagedRpcOwner::Boser => {
                let error = result.expect_err("Boser-owned operation must be rejected");
                assert_eq!(error.code, ErrorCode::Unimplemented);
            }
        }
    }
}
