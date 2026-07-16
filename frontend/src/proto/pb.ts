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

export * from '@bufbuild/protobuf/wkt';
export type { CallOptions } from '@connectrpc/connect';
export type { JsonObject, JsonValue } from '@bufbuild/protobuf';

export * from './gen/web/account_management_pb';
export * from './gen/web/alarm_pb';
export * from './gen/web/authentication_pb';
export * from './gen/web/configuration_pb';
export * from './gen/web/initial_setup_pb';
export * from './gen/web/metadata_pb';
export * from './gen/web/network_pb';
export * from './gen/web/scene_management_pb';
export * from './gen/web/shared_pb';
export * from './gen/web/system_pb';
export * from './gen/web/upgrade_pb';
export * from './gen/web/widget_data_pb';
export * from './gen/web/hardware_pb';
