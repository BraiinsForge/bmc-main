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

import { type CarbonIconType, TemperatureCelsius, TemperatureFahrenheit } from '@carbon/react/icons';

// Libs
import type { IntlShape } from 'react-intl';
import { assertUnreachable } from '@/lib/ts';
import { formatDuration } from 'date-fns';
import { isEqual } from 'es-toolkit';

// Proto
import { create } from '@bufbuild/protobuf';
import * as pb from './pb';

export function renderTimezone(tz: Maybe<pb.Timezone>): string {
    if (!tz) return 'N/A';
    return `UTC${tz.offset} (${tz.label})`;
}

export const wifiEncryptionTypeOptions: Array<Exclude<pb.EncryptionType, 0>> = [
    pb.EncryptionType.NONE,
    pb.EncryptionType.WEP,
    pb.EncryptionType.WEP_SHARED,
    pb.EncryptionType.WPA,
    pb.EncryptionType.WPA1_2,
    pb.EncryptionType.WPA2,
    pb.EncryptionType.WPA2_3,
];
export function wifiEncryptionTypeToString(intl: IntlShape, x?: Maybe<pb.EncryptionType>): null;
export function wifiEncryptionTypeToString(intl: IntlShape, x: Exclude<pb.EncryptionType, 0>): string;
export function wifiEncryptionTypeToString(intl: IntlShape, x?: null | pb.EncryptionType) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.EncryptionType.UNSPECIFIED:
            return null;

        case pb.EncryptionType.NONE:
            return formatMessage({ defaultMessage: 'None' });

        case pb.EncryptionType.WEP:
            return formatMessage({ defaultMessage: 'WEP' });

        case pb.EncryptionType.WEP_SHARED:
            return formatMessage({ defaultMessage: 'WEP Shared' });

        case pb.EncryptionType.WPA:
            return formatMessage({ defaultMessage: 'WPA' });

        case pb.EncryptionType.WPA1_2:
            return formatMessage({ defaultMessage: 'WPA / WPA2' });

        case pb.EncryptionType.WPA2:
            return formatMessage({ defaultMessage: 'WPA2' });

        case pb.EncryptionType.WPA2_3:
            return formatMessage({ defaultMessage: 'WPA2 / WPA3' });

        case pb.EncryptionType.WPA3:
            return formatMessage({ defaultMessage: 'WPA3' });

        default:
            assertUnreachable(x, 'Wifi encryption type');
    }
}

export const weekdayOptionsAll: Array<Exclude<pb.Weekday, 0>> = [
    pb.Weekday.MONDAY,
    pb.Weekday.TUESDAY,
    pb.Weekday.WEDNESDAY,
    pb.Weekday.THURSDAY,
    pb.Weekday.FRIDAY,
    pb.Weekday.SATURDAY,
    pb.Weekday.SUNDAY,
];
export const weekdayOptionsWeek: Array<Exclude<pb.Weekday, 0>> = [
    pb.Weekday.MONDAY,
    pb.Weekday.TUESDAY,
    pb.Weekday.WEDNESDAY,
    pb.Weekday.THURSDAY,
    pb.Weekday.FRIDAY,
];
export const weekdayOptionsWeekend: Array<Exclude<pb.Weekday, 0>> = [pb.Weekday.SATURDAY, pb.Weekday.SUNDAY];
export function weekdayToString(intl: IntlShape, x?: Maybe<pb.Weekday>, long?: boolean): null;
export function weekdayToString(intl: IntlShape, x: Exclude<pb.Weekday, 0>, long?: boolean): string;
export function weekdayToString(intl: IntlShape, x?: null | pb.Weekday, long?: boolean) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.Weekday.UNSPECIFIED:
            return null;

        case pb.Weekday.MONDAY:
            return long ? formatMessage({ defaultMessage: 'Monday' }) : formatMessage({ defaultMessage: 'Mon' });

        case pb.Weekday.TUESDAY:
            return long ? formatMessage({ defaultMessage: 'Tuesday' }) : formatMessage({ defaultMessage: 'Tue' });

        case pb.Weekday.WEDNESDAY:
            return long ? formatMessage({ defaultMessage: 'Wednesday' }) : formatMessage({ defaultMessage: 'Wed' });

        case pb.Weekday.THURSDAY:
            return long ? formatMessage({ defaultMessage: 'Thursday' }) : formatMessage({ defaultMessage: 'Thu' });

        case pb.Weekday.FRIDAY:
            return long ? formatMessage({ defaultMessage: 'Friday' }) : formatMessage({ defaultMessage: 'Fri' });

        case pb.Weekday.SATURDAY:
            return long ? formatMessage({ defaultMessage: 'Saturday' }) : formatMessage({ defaultMessage: 'Sat' });

        case pb.Weekday.SUNDAY:
            return long ? formatMessage({ defaultMessage: 'Sunday' }) : formatMessage({ defaultMessage: 'Sun' });

        default:
            assertUnreachable(x, 'weekday');
    }
}
export function weekdayListToString(intl: IntlShape, x: Maybe<pb.Weekday[]>): null | string {
    if (!x) return null;

    const { formatMessage } = intl;
    const unique: pb.Weekday[] = Array.from(new Set(x.filter(Boolean))).sort();

    if (isEqual(unique, weekdayOptionsAll)) return formatMessage({ defaultMessage: 'All days' });
    if (isEqual(unique, weekdayOptionsWeek)) return formatMessage({ defaultMessage: 'Weekdays' });
    if (isEqual(unique, weekdayOptionsWeekend)) return formatMessage({ defaultMessage: 'Weekends' });
    return unique
        .toSorted()
        .map(x => weekdayToString(intl, x))
        .join(', ');
}

export function alarmSnoozeOptionsToString(intl: IntlShape, snoozeOptions: Maybe<pb.SnoozeOptionsWrapper>) {
    const { formatMessage } = intl;
    return snoozeOptions?.kind?.case === 'snooze'
        ? formatMessage(
              { defaultMessage: 'On, {duration}, {limit}' },
              {
                  duration: alarmSnoozeDurationToString(intl, snoozeOptions.kind.value.duration),
                  limit: alarmSnoozeLimitToString(intl, snoozeOptions.kind.value.limit),
              },
          )
        : formatMessage({ defaultMessage: 'Off' });
}

export const alarmSnoozeLimitOptions: Array<Exclude<pb.SnoozeLimit, 0>> = [
    pb.SnoozeLimit.SNOOZE_LIMIT_FOREVER,
    pb.SnoozeLimit.SNOOZE_LIMIT_3,
    pb.SnoozeLimit.SNOOZE_LIMIT_5,
];
export function alarmSnoozeLimitToString(intl: IntlShape, x?: Maybe<pb.SnoozeLimit>): null;
export function alarmSnoozeLimitToString(intl: IntlShape, x: Exclude<pb.SnoozeLimit, 0>): string;
export function alarmSnoozeLimitToString(intl: IntlShape, x?: null | pb.SnoozeLimit) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.SnoozeLimit.SNOOZE_LIMIT_UNSPECIFIED:
            return null;

        case pb.SnoozeLimit.SNOOZE_LIMIT_FOREVER:
            return formatMessage({ defaultMessage: 'Forever' });

        case pb.SnoozeLimit.SNOOZE_LIMIT_3:
            return formatMessage({ defaultMessage: '3 snoozes' });

        case pb.SnoozeLimit.SNOOZE_LIMIT_5:
            return formatMessage({ defaultMessage: '5 snoozes' });

        default:
            assertUnreachable(x, 'alarm snooze limit');
    }
}

export const alarmSnoozeDurationOptions: Array<Exclude<pb.SnoozeDuration, 0>> = [
    pb.SnoozeDuration.SNOOZE_DURATION_5_MINUTES,
    pb.SnoozeDuration.SNOOZE_DURATION_10_MINUTES,
    pb.SnoozeDuration.SNOOZE_DURATION_15_MINUTES,
    pb.SnoozeDuration.SNOOZE_DURATION_30_MINUTES,
];
export function alarmSnoozeDurationToString(intl: IntlShape, x?: Maybe<pb.SnoozeDuration>): null;
export function alarmSnoozeDurationToString(intl: IntlShape, x: Exclude<pb.SnoozeDuration, 0>): string;
export function alarmSnoozeDurationToString(intl: IntlShape, x?: null | pb.SnoozeDuration) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.SnoozeDuration.SNOOZE_DURATION_UNSPECIFIED:
            return null;

        case pb.SnoozeDuration.SNOOZE_DURATION_5_MINUTES:
            return formatMessage({ defaultMessage: '5 minutes' });

        case pb.SnoozeDuration.SNOOZE_DURATION_10_MINUTES:
            return formatMessage({ defaultMessage: '10 minutes' });

        case pb.SnoozeDuration.SNOOZE_DURATION_15_MINUTES:
            return formatMessage({ defaultMessage: '15 minutes' });

        case pb.SnoozeDuration.SNOOZE_DURATION_30_MINUTES:
            return formatMessage({ defaultMessage: '30 minutes' });

        default:
            assertUnreachable(x, 'alarm snooze duration');
    }
}

export const sceneCyclingEffectOptions: Array<Exclude<pb.SceneCyclingTransition, 0>> = [
    pb.SceneCyclingTransition.SLIDE,
    pb.SceneCyclingTransition.FADE,
    pb.SceneCyclingTransition.NONE,
];
export function sceneCyclingEffectToString(intl: IntlShape, x?: Maybe<pb.SceneCyclingTransition>): null;
export function sceneCyclingEffectToString(intl: IntlShape, x: Exclude<pb.SceneCyclingTransition, 0>): string;
export function sceneCyclingEffectToString(intl: IntlShape, x?: null | pb.SceneCyclingTransition) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.SceneCyclingTransition.UNSPECIFIED:
            return null;

        case pb.SceneCyclingTransition.FADE:
            return formatMessage({ defaultMessage: 'Fade' });

        case pb.SceneCyclingTransition.SLIDE:
            return formatMessage({ defaultMessage: 'Slide' });

        case pb.SceneCyclingTransition.NONE:
            return formatMessage({ defaultMessage: 'None' });

        default:
            assertUnreachable(x, 'Widget transition effect');
    }
}

export const sceneCycleDurationOptions: number[] = [10, 20, 30, 40, 50, 60, 90, 120];
export function sceneCycleDurationToString(value: Maybe<number>): string {
    if (value == null) return 'N/A';
    const minutes = Math.floor(value / 60);
    const seconds = Math.floor(value - minutes * 60);
    return formatDuration({ minutes, seconds }, { format: ['minutes', 'seconds'] });
}

/**
 * The selection dropdown is a Downshift instance,
 * which requires all options to be of the same type.
 *
 * This is a placeholder object that represents the "Other…" option.
 * It needs to be caught and handled in special way in…
 *  - renderToString
 *  - renderToElement
 *  - onChange
 */
export const WIFI_AP_OTHER_PLACEHOLDER: Readonly<pb.WifiNetwork> = Object.freeze(
    create(pb.WifiNetworkSchema, { ssid: 'WIFI_AP_OTHER_PLACEHOLDER' }),
);
export function wifiNetworkToString(net: Maybe<pb.WifiNetwork>, other: string): null | string {
    if (net == null) return null;
    return net.ssid === WIFI_AP_OTHER_PLACEHOLDER.ssid ? other : net.ssid;
}

export const dateFormatOptions: Array<Exclude<pb.DateFormat, 0>> = [
    pb.DateFormat.DD_MM_YYYY_DOT,
    pb.DateFormat.DD_MM_YYYY_SLASH,
    pb.DateFormat.D_M_YYYY_SLASH,
    pb.DateFormat.M_D_YYYY_SLASH,
    pb.DateFormat.DD_MM_YYYY_DASH,
    pb.DateFormat.YYYY_M_D_SLASH,
    pb.DateFormat.YYYY_MM_DD_DOT,
    pb.DateFormat.YYYY_MM_DD_DASH,
];
export function dateFormatToString(x: Maybe<pb.DateFormat>): null;
export function dateFormatToString(x: Exclude<pb.DateFormat, 0>): string;
export function dateFormatToString(x: Maybe<pb.DateFormat>) {
    switch (x) {
        case null:
        case undefined:
        case pb.DateFormat.UNSPECIFIED:
            return null;

        case pb.DateFormat.DD_MM_YYYY_DOT:
            return 'DD.MM.YYYY' as string;

        case pb.DateFormat.DD_MM_YYYY_SLASH:
            return 'DD/MM/YYYY';

        case pb.DateFormat.D_M_YYYY_SLASH:
            return 'D/M/YYYY';

        case pb.DateFormat.M_D_YYYY_SLASH:
            return 'M/D/YYYY';

        case pb.DateFormat.DD_MM_YYYY_DASH:
            return 'DD-MM-YYYY';

        case pb.DateFormat.YYYY_M_D_SLASH:
            return 'YYYY/M/D';

        case pb.DateFormat.YYYY_MM_DD_DOT:
            return 'YYYY.MM.DD';

        case pb.DateFormat.YYYY_MM_DD_DASH:
            return 'YYYY-MM-DD';

        default:
            assertUnreachable(x, 'date format');
    }
}

export const temperatureUnitOptions: Map<Exclude<pb.TemperatureUnit, 0>, CarbonIconType> = new Map([
    [pb.TemperatureUnit.CELSIUS, TemperatureCelsius],
    [pb.TemperatureUnit.FAHRENHEIT, TemperatureFahrenheit],
]);
export function temperatureUnitToString(intl: IntlShape, x?: Maybe<pb.TemperatureUnit>): null;
export function temperatureUnitToString(intl: IntlShape, x: Exclude<pb.TemperatureUnit, 0>): string;
export function temperatureUnitToString(intl: IntlShape, x?: null | pb.TemperatureUnit) {
    switch (x) {
        case null:
        case undefined:
        case pb.TemperatureUnit.UNSPECIFIED:
            return null;

        case pb.TemperatureUnit.CELSIUS:
            return intl.formatMessage({ defaultMessage: 'Celsius' });

        case pb.TemperatureUnit.FAHRENHEIT:
            return intl.formatMessage({ defaultMessage: 'Fahrenheit' });

        default:
            assertUnreachable(x, 'temperature unit');
    }
}

export const unitSystemOptions: Array<Exclude<pb.UnitSystem, 0>> = [pb.UnitSystem.METRIC, pb.UnitSystem.IMPERIAL];
export function unitSystemToString(intl: IntlShape, x?: Maybe<pb.UnitSystem>): null;
export function unitSystemToString(intl: IntlShape, x: Exclude<pb.UnitSystem, 0>): string;
export function unitSystemToString(intl: IntlShape, x?: null | pb.UnitSystem) {
    switch (x) {
        case null:
        case undefined:
        case pb.UnitSystem.UNSPECIFIED:
            return null;

        case pb.UnitSystem.METRIC:
            return intl.formatMessage({ defaultMessage: 'Metric (km, kg)' });

        case pb.UnitSystem.IMPERIAL:
            return intl.formatMessage({ defaultMessage: 'Imperial (mi, lb)' });

        default:
            assertUnreachable(x, 'unit system');
    }
}

export const numberFormatOptions: Array<Exclude<pb.NumberFormat, 0>> = [
    pb.NumberFormat.SPACE_GROUP_COMMA_DECIMAL,
    pb.NumberFormat.COMMA_GROUP_DOT_DECIMAL,
    pb.NumberFormat.DOT_GROUP_COMMA_DECIMAL,
    pb.NumberFormat.SPACE_GROUP_DOT_DECIMAL,
];
export function numberFormatToString(x: Maybe<pb.NumberFormat>): null | string {
    if (!x) return null;
    switch (x) {
        case pb.NumberFormat.SPACE_GROUP_COMMA_DECIMAL:
            return '1 234 567,89';

        case pb.NumberFormat.COMMA_GROUP_DOT_DECIMAL:
            return '1,234,567.89';

        case pb.NumberFormat.DOT_GROUP_COMMA_DECIMAL:
            return '1.234.567,89';

        case pb.NumberFormat.SPACE_GROUP_DOT_DECIMAL:
            return '1 234 567.89';

        default:
            assertUnreachable(x, 'number format');
    }
}

// ── Manifest-driven scene / widget helpers ────────────────────────────
//
// Since every widget is identified by a manifest UID, titles and
// descriptions come from the manifest registry.  Callers pass a lookup
// (typically a Map<string, WidgetManifest>) so we can resolve UIDs
// synchronously per render.

export type ManifestLookup = Map<string, pb.WidgetManifest>;

/** Credential types keyed by id, so resolving a slot's type costs nothing per render. */
export type CredentialTypeLookup = Map<pb.CredentialType['id'], pb.CredentialType>;

/** Display title for a widget — prefers the manifest's human-readable name. */
export function widgetTitle(widget: Maybe<pb.Widget>, manifests: Maybe<ManifestLookup>): null | string {
    if (!widget) return null;
    return manifests?.get(widget.config?.widgetUid ?? '')?.name || null;
}

/** Short description of a widget — uses the manifest's description text. */
export function widgetDescription(widget: Maybe<pb.Widget>, manifests: Maybe<ManifestLookup>): null | string {
    if (!widget) return null;
    return manifests?.get(widget.config?.widgetUid ?? '')?.description || null;
}

/** Title for a scene — names the fullscreen widget, or a fixed label for combined scenes. */
export function sceneTitle(intl: IntlShape, scene: Maybe<pb.Scene>, manifests: Maybe<ManifestLookup>): null | string {
    if (!scene) return null;
    const { case: sceneKindCase } = scene.kind;
    switch (sceneKindCase) {
        case undefined:
            return null;
        case 'combined':
            return intl.formatMessage({ defaultMessage: 'Combined Scene' });
        case 'fullscreen':
            return widgetTitle(scene.kind.value.widget, manifests);
        default:
            assertUnreachable(sceneKindCase, 'scene kind');
    }
}

/** Description of a scene — widget list for combined, single description for fullscreen. */
export function sceneDescription(scene: Maybe<pb.Scene>, manifests: Maybe<ManifestLookup>): null | string {
    if (!scene) return null;
    const { case: sceneKindCase } = scene.kind;
    switch (sceneKindCase) {
        case undefined:
            return null;
        case 'combined':
            return scene.kind.value.widgets
                .map(w => widgetTitle(w, manifests))
                .filter((x): x is string => !!x)
                .join(', ');
        case 'fullscreen':
            return widgetDescription(scene.kind.value.widget, manifests);
        default:
            assertUnreachable(sceneKindCase, 'scene kind');
    }
}

// ── Optimistic clone placeholder ──────────────────────────────────────
//
// Cloning a scene inserts a placeholder row immediately for snappy feedback,
// before the debounced reload swaps in the real backend scene.
//
// The placeholder needs a unique id (so it gets a distinct React key
// — no key collision, no doubling on repeated clicks, clean reconcile on reload)
// that is also recognizable, so the UI can disable controls that act by id
// while the row is not yet a real backend scene.

export const OPTIMISTIC_SCENE_ID_PREFIX = '__bmc-optimistic-clone__:';

/** Build a unique placeholder scene id from a monotonic sequence value. */
export function optimisticSceneId(seq: number): string {
    return `${OPTIMISTIC_SCENE_ID_PREFIX}${seq}`;
}

/** True for placeholder rows inserted by an in-flight clone (not yet on the backend). */
export function isOptimisticSceneId(id: string): boolean {
    return id.startsWith(OPTIMISTIC_SCENE_ID_PREFIX);
}
