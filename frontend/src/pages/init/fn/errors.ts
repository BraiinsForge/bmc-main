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

import * as pb from '@/proto';
import type { FormPropsToLocalState } from '@/lib/form';
import type { MiningSetupProps, NetworkProtocol, SetupProps } from '../components/Setup';

// The form carries fields for both variants; only the ones relevant to the
// active device capabilities are rendered and submitted.
export type FormState = FormPropsToLocalState<SetupProps & MiningSetupProps>;
export type FormKey = keyof FormState['values'];

type ProtoField<T> = Exclude<keyof T, '$typeName' | '$unknown'>;
// NOTE: field paths as the backend names them in its violations: the request
// message's own fields and the nested pool and network ones. Typed
// from the generated messages, so a proto rename fails the build here.
type PoolPath = Extract<ProtoField<pb.SettingsRequest>, 'pool'>;
type NetworkPath = Extract<ProtoField<pb.SettingsRequest>, 'network'>;
type StaticPath = `${NetworkPath}.${Extract<NetworkProtocol, 'static'>}`;
type WirePath =
    | ProtoField<pb.SettingsRequest>
    | `${PoolPath}.${ProtoField<pb.PoolConfig>}`
    | `${NetworkPath}.${ProtoField<pb.NetworkConfig>}`
    | `${StaticPath}.${ProtoField<pb.NetworkConfigStatic>}`;

export const FORM_KEY_BY_WIRE_PATH = {
    password: 'password1',
    timezoneId: 'timezone',
    timeFormat: 'timeFormat',
    dateFormat: 'dateFormat',
    numberFormat: 'numberFormat',
    temperatureUnit: 'temperatureUnits',
    unitSystem: 'unitSystem',
    hostname: 'hostname',
    'pool.url': 'poolUrl',
    'pool.user': 'poolUser',
    'network.protocol': 'protocol',
    'network.static.address': 'staticAddress',
    'network.static.netmask': 'staticNetmask',
    'network.static.gateway': 'staticGateway',
    'network.static.dnsServers': 'staticDns',
} as const satisfies Partial<Record<WirePath, FormKey>>;

/**
 * Maps a `SetupDevice` failure onto the flat form: violations arrive nested
 * the way the request message is, and ones naming no form field go global.
 */
export function toFormErrors(exception: unknown): FormState['errors'] {
    const { global, fields } = pb.parseFormErrors(exception);
    const mapped: Partial<Record<FormKey, string[]>> = {};

    const visit = (node: unknown, path: string): void => {
        if (Array.isArray(node)) {
            const messages = node.flat(Number.POSITIVE_INFINITY).filter((x): x is string => typeof x === 'string');
            const key = FORM_KEY_BY_WIRE_PATH[path as keyof typeof FORM_KEY_BY_WIRE_PATH];
            if (key) mapped[key] = [...(mapped[key] ?? []), ...messages];
            else global.push(...messages);
            return;
        }
        if (node && typeof node === 'object') {
            for (const [name, child] of Object.entries(node)) visit(child, path ? `${path}.${name}` : name);
        }
    };
    visit(fields, '');

    return { global, fields: mapped };
}
