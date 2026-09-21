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
use crate::backlight::DisplayBacklightDriver;
use crate::bootloader_config::BootloaderConfig;
use crate::compositor::testing::RecordingCompositor;
use crate::session;
use crate::{App, BmcManager, Configuration, UpgradeError, UpgradeMarker};
use axum_extra::extract::cookie::Cookie;
use bmc_button::{ButtonEventStream, Buttons};
use bmc_grpc::web::{
    ChangePasswordRequest, CheckForUpgradeRequest, CreatePasswordRequest, GetTimezoneListResponse,
    GetTimezoneResponse, NetworkConfig, NetworkInfoResponse, RemovePasswordRequest,
    ScanWifiResponse, SetAutoUpgradeRequest, SetTimezoneRequest, SetWifiRequest, SettingsRequest,
    StartUpgradeRequest, WifiSavedNetworksResponse, WifiStatusResponse,
    initial_setup_service_client::InitialSetupServiceClient,
    network_service_client::NetworkServiceClient,
    network_service_server::{NetworkService, NetworkServiceServer},
    system_service_client::SystemServiceClient,
    system_service_server::{SystemService, SystemServiceServer},
    upgrade_service_client::UpgradeServiceClient,
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
use prost::Message;
use reqwest::Client;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, watch};
use tonic::{Code, Response};

const UNREACHABLE: &str = "BUG: session manager is not called by these tests";
const GATED_ROUTE_UNREACHABLE: &str = "BUG: gated production route reached its stub";

#[derive(Clone, Debug)]
struct StubSession;

impl session::Handle for StubSession {
    fn is_valid(&self) -> bool {
        true
    }

    fn id(&self) -> String {
        "test-session".to_owned()
    }
}

#[derive(Debug, Default)]
struct StubSessionManager;

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
struct StubBmcManager {
    network_manager: MockNetworkManager,
    timezone_sender: watch::Sender<Timezone>,
}

impl Default for StubBmcManager {
    fn default() -> Self {
        Self {
            network_manager: MockNetworkManager::with_provisioning(false, true),
            timezone_sender: watch::channel(Timezone::default()).0,
        }
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
    fn record(&self) -> Response<()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Response::new(())
    }
}

#[async_trait::async_trait]
impl SystemService for RecordingSystemService {
    async fn has_password(&self, _request: tonic::Request<()>) -> Result<Response<bool>, Status> {
        Ok(Response::new(false))
    }

    async fn create_password(
        &self,
        _request: tonic::Request<CreatePasswordRequest>,
    ) -> Result<Response<()>, Status> {
        Ok(self.record())
    }

    async fn change_password(
        &self,
        _request: tonic::Request<ChangePasswordRequest>,
    ) -> Result<Response<()>, Status> {
        Ok(self.record())
    }

    async fn remove_password(
        &self,
        _request: tonic::Request<RemovePasswordRequest>,
    ) -> Result<Response<()>, Status> {
        Ok(self.record())
    }

    async fn get_timezone(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<Response<GetTimezoneResponse>, Status> {
        Ok(Response::new(GetTimezoneResponse::default()))
    }

    async fn set_timezone(
        &self,
        _request: tonic::Request<SetTimezoneRequest>,
    ) -> Result<Response<()>, Status> {
        Ok(self.record())
    }

    async fn get_timezone_list(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<Response<GetTimezoneListResponse>, Status> {
        Ok(Response::new(GetTimezoneListResponse::default()))
    }

    async fn factory_reset(&self, _request: tonic::Request<()>) -> Result<Response<()>, Status> {
        Ok(self.record())
    }

    async fn reboot(&self, _request: tonic::Request<()>) -> Result<Response<()>, Status> {
        Ok(self.record())
    }
}

#[derive(Clone, Default)]
struct RecordingNetworkService {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl NetworkService for RecordingNetworkService {
    async fn get_network_info(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<Response<NetworkInfoResponse>, Status> {
        Ok(Response::new(NetworkInfoResponse::default()))
    }

    async fn get_network_config(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<Response<NetworkConfig>, Status> {
        Ok(Response::new(NetworkConfig::default()))
    }

    async fn set_network_config(
        &self,
        _request: tonic::Request<NetworkConfig>,
    ) -> Result<Response<()>, Status> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Response::new(()))
    }

    async fn get_wifi_status(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<Response<WifiStatusResponse>, Status> {
        Ok(Response::new(WifiStatusResponse::default()))
    }

    async fn get_wifi_saved_networks(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<Response<WifiSavedNetworksResponse>, Status> {
        Ok(Response::new(WifiSavedNetworksResponse::default()))
    }

    async fn set_wifi(
        &self,
        _request: tonic::Request<SetWifiRequest>,
    ) -> Result<Response<()>, Status> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Response::new(()))
    }

    async fn scan_wifi(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<Response<ScanWifiResponse>, Status> {
        Ok(Response::new(ScanWifiResponse::default()))
    }
}

fn capabilities(product: Product) -> HardwareCapabilities {
    HardwareProfile::for_product(product).capabilities()
}

fn auth_interceptor() -> AuthInterceptor<StubSessionManager> {
    AuthInterceptor {
        session_manager: Arc::new(StubSessionManager),
    }
}

fn ownership_interceptor(product: Product) -> BoserOwnershipInterceptor {
    BoserOwnershipInterceptor {
        hardware_capabilities: capabilities(product),
    }
}

fn service_method_paths(service_name: &str) -> Vec<String> {
    let descriptor_set = prost_types::FileDescriptorSet::decode(web::FILE_DESCRIPTOR_SET)
        .expect("BUG: generated gRPC descriptor set must decode");

    for file in descriptor_set.file {
        let package = file
            .package
            .expect("BUG: generated gRPC file must declare its package");
        for service in file.service {
            let name = service
                .name
                .expect("BUG: generated gRPC service must have a name");
            if format!("{package}.{name}") == service_name {
                return service
                    .method
                    .into_iter()
                    .map(|method| {
                        let method = method
                            .name
                            .expect("BUG: generated gRPC method must have a name");
                        format!("/{service_name}/{method}")
                    })
                    .collect();
            }
        }
    }

    panic!("BUG: generated gRPC service {service_name} is missing");
}

fn authenticated<T>(message: T) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.extensions_mut().insert(StubSession);
    request
}

async fn production_routes(product: Product) -> (tempfile::TempDir, Routes) {
    let tempdir = tempfile::tempdir().expect("BUG: test tempdir creation must succeed");
    let config = Configuration {
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
    let manager = Arc::new(StubBmcManager::default());
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
        Arc::new(RecordingCompositor::with_hardware_capabilities(
            capabilities(product),
        )),
        None,
    )
    .await
    .expect("managed test application must initialize");

    (tempdir, app.build_grpc_routes())
}

fn ownership_intercepted_routes(
    product: Product,
    system: RecordingSystemService,
    network: RecordingNetworkService,
) -> Routes {
    let auth_interceptor = auth_interceptor();
    let ownership_interceptor = ownership_interceptor(product);
    Routes::new(authenticated_with_boser_ownership(
        SystemServiceServer::new(system),
        ownership_interceptor,
        auth_interceptor.clone(),
    ))
    .add_service(authenticated_with_boser_ownership(
        NetworkServiceServer::new(network),
        ownership_interceptor,
        auth_interceptor,
    ))
}

async fn call_boser_owned_mutations(routes: Routes) -> Vec<Result<(), Status>> {
    let mut system = SystemServiceClient::new(routes.clone());
    let mut network = NetworkServiceClient::new(routes);

    vec![
        system
            .create_password(authenticated(CreatePasswordRequest::default()))
            .await
            .map(|_| ()),
        system
            .change_password(authenticated(ChangePasswordRequest::default()))
            .await
            .map(|_| ()),
        system
            .remove_password(authenticated(RemovePasswordRequest::default()))
            .await
            .map(|_| ()),
        system
            .set_timezone(authenticated(SetTimezoneRequest::default()))
            .await
            .map(|_| ()),
        system.factory_reset(authenticated(())).await.map(|_| ()),
        system.reboot(authenticated(())).await.map(|_| ()),
        network
            .set_network_config(authenticated(NetworkConfig::default()))
            .await
            .map(|_| ()),
        network
            .set_wifi(authenticated(SetWifiRequest::default()))
            .await
            .map(|_| ()),
    ]
}

#[tokio::test]
async fn production_routes_gate_system_and_network_mutations() {
    let (_tempdir, routes) = production_routes(Product::Bfm100).await;
    let mut system = SystemServiceClient::new(routes.clone());
    let mut network = NetworkServiceClient::new(routes);

    let results = [
        system.reboot(authenticated(())).await.map(|_| ()),
        network
            .set_wifi(authenticated(SetWifiRequest::default()))
            .await
            .map(|_| ()),
    ];

    for result in results {
        let status = result.expect_err("managed production route must reject mutation");
        assert_eq!(status.code(), Code::Unimplemented);
        assert_eq!(status.message(), BOSER_MANAGED_STATUS_MESSAGE);
    }
}

#[tokio::test]
async fn production_routes_preserve_self_managed_network_configuration() {
    let (_tempdir, routes) = production_routes(Product::Bmc100).await;
    let mut network = NetworkServiceClient::new(routes);

    network
        .set_network_config(authenticated(NetworkConfig {
            protocol: Some(bmc_grpc::web::network_config::Protocol::Dhcp(())),
        }))
        .await
        .expect("BUG: self-managed production route must reach the network handler");
}

#[tokio::test]
async fn managed_production_routes_keep_initial_setup_available() {
    let (_tempdir, routes) = production_routes(Product::Bfm100).await;
    let mut initial_setup = InitialSetupServiceClient::new(routes);

    let settings = initial_setup
        .get_settings_data(())
        .await
        .expect("managed production route must reach the initial-setup handler")
        .into_inner();

    assert!(
        !settings.timezones.is_empty(),
        "initial setup must return its timezone choices"
    );

    let status = initial_setup
        .setup_device(SettingsRequest::default())
        .await
        .expect_err("an invalid setup request must reach field validation");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn managed_production_routes_reject_every_upgrade_rpc() {
    let (_tempdir, routes) = production_routes(Product::Bfm100).await;
    let mut upgrade = UpgradeServiceClient::new(routes);

    let results = [
        upgrade
            .check_for_upgrade(authenticated(CheckForUpgradeRequest::default()))
            .await
            .map(|_| ()),
        upgrade
            .get_installable_widgets(authenticated(()))
            .await
            .map(|_| ()),
        upgrade
            .start_upgrade(authenticated(StartUpgradeRequest::default()))
            .await
            .map(|_| ()),
        upgrade
            .set_auto_upgrade(authenticated(SetAutoUpgradeRequest::default()))
            .await
            .map(|_| ()),
        upgrade
            .get_auto_upgrade(authenticated(()))
            .await
            .map(|_| ()),
    ];

    for result in results {
        let status = result.expect_err("managed upgrade RPC must be rejected");
        assert_eq!(status.code(), Code::Unimplemented);
        assert_eq!(status.message(), BOSER_MANAGED_STATUS_MESSAGE);
    }
}

#[tokio::test]
async fn self_managed_production_routes_keep_upgrade_available() {
    let (_tempdir, routes) = production_routes(Product::Bmc100).await;
    let mut upgrade = UpgradeServiceClient::new(routes);

    upgrade
        .get_auto_upgrade(authenticated(()))
        .await
        .expect("self-managed production route must reach the upgrade handler");
}

#[tokio::test]
async fn managed_mutations_are_rejected_before_handlers() {
    let system = RecordingSystemService::default();
    let network = RecordingNetworkService::default();
    let calls = [system.calls.clone(), network.calls.clone()];

    let results = call_boser_owned_mutations(ownership_intercepted_routes(
        Product::Bfm100,
        system,
        network,
    ))
    .await;

    assert_eq!(results.len(), 8);
    for result in results {
        let status = result.expect_err("managed mutation must be rejected");
        assert_eq!(status.code(), Code::Unimplemented);
        assert_eq!(status.message(), BOSER_MANAGED_STATUS_MESSAGE);
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

    let results = call_boser_owned_mutations(ownership_intercepted_routes(
        Product::Bmc100,
        system,
        network,
    ))
    .await;

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
    let routes = ownership_intercepted_routes(
        Product::Bfm100,
        RecordingSystemService::default(),
        RecordingNetworkService::default(),
    );
    let mut system = SystemServiceClient::new(routes);

    let status = system
        .reboot(tonic::Request::new(()))
        .await
        .expect_err("missing session must be rejected first");

    assert_eq!(status.code(), Code::Unauthenticated);
}

#[tokio::test]
async fn auth_interceptor_does_not_enforce_ownership() {
    let mut request = Request::builder()
        .uri(format!(
            "/{}/Reboot",
            web::system_service_server::SERVICE_NAME
        ))
        .body(Body::empty())
        .expect("BUG: static gRPC request is valid");
    request.extensions_mut().insert(StubSession);

    auth_interceptor()
        .intercept(request)
        .await
        .expect("authentication must not enforce Boser ownership");
}

const EXPECTED_MANAGED_RPC_OWNERS: [(&str, &str, ManagedRpcOwner); 21] = [
    (
        web::system_service_server::SERVICE_NAME,
        "HasPassword",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::system_service_server::SERVICE_NAME,
        "CreatePassword",
        ManagedRpcOwner::Boser,
    ),
    (
        web::system_service_server::SERVICE_NAME,
        "ChangePassword",
        ManagedRpcOwner::Boser,
    ),
    (
        web::system_service_server::SERVICE_NAME,
        "RemovePassword",
        ManagedRpcOwner::Boser,
    ),
    (
        web::system_service_server::SERVICE_NAME,
        "GetTimezone",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::system_service_server::SERVICE_NAME,
        "SetTimezone",
        ManagedRpcOwner::Boser,
    ),
    (
        web::system_service_server::SERVICE_NAME,
        "GetTimezoneList",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::system_service_server::SERVICE_NAME,
        "FactoryReset",
        ManagedRpcOwner::Boser,
    ),
    (
        web::system_service_server::SERVICE_NAME,
        "Reboot",
        ManagedRpcOwner::Boser,
    ),
    (
        web::network_service_server::SERVICE_NAME,
        "GetNetworkInfo",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::network_service_server::SERVICE_NAME,
        "GetNetworkConfig",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::network_service_server::SERVICE_NAME,
        "SetNetworkConfig",
        ManagedRpcOwner::Boser,
    ),
    (
        web::network_service_server::SERVICE_NAME,
        "GetWifiStatus",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::network_service_server::SERVICE_NAME,
        "GetWifiSavedNetworks",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::network_service_server::SERVICE_NAME,
        "SetWifi",
        ManagedRpcOwner::Boser,
    ),
    (
        web::network_service_server::SERVICE_NAME,
        "ScanWifi",
        ManagedRpcOwner::Bmc,
    ),
    (
        web::upgrade_service_server::SERVICE_NAME,
        "CheckForUpgrade",
        ManagedRpcOwner::Boser,
    ),
    (
        web::upgrade_service_server::SERVICE_NAME,
        "GetInstallableWidgets",
        ManagedRpcOwner::Boser,
    ),
    (
        web::upgrade_service_server::SERVICE_NAME,
        "StartUpgrade",
        ManagedRpcOwner::Boser,
    ),
    (
        web::upgrade_service_server::SERVICE_NAME,
        "SetAutoUpgrade",
        ManagedRpcOwner::Boser,
    ),
    (
        web::upgrade_service_server::SERVICE_NAME,
        "GetAutoUpgrade",
        ManagedRpcOwner::Boser,
    ),
];

#[test]
fn every_ownership_intercepted_service_method_has_the_expected_owner() {
    let mut actual_paths = [
        web::system_service_server::SERVICE_NAME,
        web::network_service_server::SERVICE_NAME,
        web::upgrade_service_server::SERVICE_NAME,
    ]
    .into_iter()
    .flat_map(service_method_paths)
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

#[tokio::test]
async fn ownership_interceptor_enforces_every_managed_rpc_owner() {
    for (service, method, expected_owner) in EXPECTED_MANAGED_RPC_OWNERS {
        let path = format!("/{service}/{method}");
        let request = Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("BUG: static gRPC request is valid");
        let result = ownership_interceptor(Product::Bfm100)
            .intercept(request)
            .await;
        match expected_owner {
            ManagedRpcOwner::Bmc => {
                result.expect("BMC-owned operation must remain available");
            }
            ManagedRpcOwner::Boser => {
                let status = result.expect_err("Boser-owned operation must be rejected");
                assert_eq!(status.code(), Code::Unimplemented);
            }
        }
    }
}
