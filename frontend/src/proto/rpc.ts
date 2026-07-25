// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { getClient } from './transport';

import {
    AccountManagementService,
    AlarmService,
    AuthenticationService,
    ConfigurationService,
    CredentialManagementService,
    HardwareService,
    InitialSetupService,
    MetadataService,
    NetworkService,
    SceneManagementService,
    SystemService,
    UpgradeService,
} from './pb';

// Utils
export const rpc = {
    init: getClient(InitialSetupService),

    accounts: getClient(AccountManagementService),
    credentials: getClient(CredentialManagementService),
    auth: getClient(AuthenticationService),
    alarm: getClient(AlarmService),
    config: getClient(ConfigurationService),
    hardware: getClient(HardwareService),
    meta: getClient(MetadataService),
    net: getClient(NetworkService),
    sys: getClient(SystemService),
    upgrade: getClient(UpgradeService),
    scenes: getClient(SceneManagementService),
};

// Export services
export const services = {
    AccountManagementService,
    AuthenticationService,
    CredentialManagementService,
    HardwareService,
    InitialSetupService,
    MetadataService,
    SceneManagementService,
    SystemService,
    UpgradeService,
};
