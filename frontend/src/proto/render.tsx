import type { IntlShape } from 'react-intl';
import { assertUnreachable } from '@/lib/ts';

import { create } from '@bufbuild/protobuf';
import * as pb from './pb';

export function renderTimezone(tz: Maybe<pb.Timezone>): string {
    if (!tz) return 'N/A';
    return `UTC${tz.offset} (${tz.label})`;
}

export enum SceneCycleEffect {
    Slide = 'Slide',
    Fade = 'Fade',
    Scale = 'Scale',
    Rotate = 'Rotate',
    Translate = 'Translate',
    Morphing = 'Morphing',
}

export const sceneCycleEffects: SceneCycleEffect[] = [
    SceneCycleEffect.Slide,
    SceneCycleEffect.Fade,
    SceneCycleEffect.Scale,
    SceneCycleEffect.Rotate,
    SceneCycleEffect.Translate,
    SceneCycleEffect.Morphing,
];
export function sceneCycleEffectToString(intl: IntlShape, v: Maybe<SceneCycleEffect>): string {
    if (v == null) return 'N/A';

    switch (v) {
        case SceneCycleEffect.Slide:
            return intl.formatMessage({ defaultMessage: 'Slide' });
        case SceneCycleEffect.Fade:
            return intl.formatMessage({ defaultMessage: 'Fade' });
        case SceneCycleEffect.Scale:
            return intl.formatMessage({ defaultMessage: 'Scale' });
        case SceneCycleEffect.Rotate:
            return intl.formatMessage({ defaultMessage: 'Rotate' });
        case SceneCycleEffect.Translate:
            return intl.formatMessage({ defaultMessage: 'Translate' });
        case SceneCycleEffect.Morphing:
            return intl.formatMessage({ defaultMessage: 'Morphing' });

        default:
            assertUnreachable(v, 'Scene cycle effect');
    }
}

export const wifiEncryptionTypeOptions: Array<Exclude<pb.EncryptionType, pb.EncryptionType.UNSPECIFIED>> = [
    pb.EncryptionType.NONE,
    pb.EncryptionType.WEP,
    pb.EncryptionType.WEP_SHARED,
    pb.EncryptionType.WPA,
    pb.EncryptionType.WPA1_2,
    pb.EncryptionType.WPA2,
    pb.EncryptionType.WPA2_3,
];
export function wifiEncryptionTypeToString(intl: IntlShape, x: pb.EncryptionType): null | string {
    const { formatMessage } = intl;

    switch (x) {
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
export function dateFormatToString(x: Maybe<pb.DateFormat>): null | string {
    if (!x) return null;

    switch (x) {
        case pb.DateFormat.DD_MM_YYYY_DOT:
            return 'DD.MM.YYYY';

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

export function clockStyleToString(intl: IntlShape, style: pb.ClockWidget_ClockStyle): null | string {
    if (!style) return null;
    switch (style) {
        case pb.ClockWidget_ClockStyle.ANALOG_ROUND:
            return intl.formatMessage({ defaultMessage: 'Analog (round)' });

        case pb.ClockWidget_ClockStyle.ANALOG_RECT:
            return intl.formatMessage({ defaultMessage: 'Analog (rectangular)' });

        case pb.ClockWidget_ClockStyle.DIGITAL:
            return intl.formatMessage({ defaultMessage: 'Digital' });

        default:
            assertUnreachable(style, 'clock style');
    }
}
export function fontStyleToString(intl: IntlShape, style: pb.FontStyle): null | string {
    if (!style) return null;
    switch (style) {
        case pb.FontStyle.LIGHT:
            return intl.formatMessage({ defaultMessage: 'Light' });

        case pb.FontStyle.MEDIUM:
            return intl.formatMessage({ defaultMessage: 'Medium' });

        case pb.FontStyle.BOLD:
            return intl.formatMessage({ defaultMessage: 'Bold' });

        default:
            assertUnreachable(style, 'clock style');
    }
}

export function sceneTitle(intl: IntlShape, kind: Maybe<ProtoOneofCase<pb.WidgetKind['value']>>): null | string {
    switch (kind) {
        case null:
        case undefined:
            return null;

        case 'clock':
            return intl.formatMessage({ defaultMessage: 'Clock' });

        default:
            assertUnreachable(kind);
    }
}

export function widgetDescription(intl: IntlShape, data: Maybe<pb.WidgetKind>) {
    const val = data?.value;
    switch (val?.case) {
        case undefined:
            return null;

        case 'clock':
            return intl.formatMessage(
                { defaultMessage: 'Style: {style}, font: {font}' },
                {
                    style: clockStyleToString(intl, val.value.clockStyle) || 'N/A',
                    font: fontStyleToString(intl, val.value.numbersFontStyle) || 'N/A',
                },
            );

        default:
            assertUnreachable(val, 'fullscreen widget kind');
    }
}
export function sceneDescription(intl: IntlShape, data: Maybe<MaybeArray<pb.Widget>>): null | string {
    if (data == null) return null;

    // Combined scene
    if (Array.isArray(data)) {
        return data
            .map(x => sceneTitle(intl, x.kind?.value.case))
            .filter(Boolean)
            .join(', ');
    }

    // Fullscreen widget
    else if (data) return widgetDescription(intl, data.kind);
    // Fail
    else assertUnreachable(data);
}
